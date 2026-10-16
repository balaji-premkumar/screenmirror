import 'dart:async';

import 'package:flutter/foundation.dart';

import '../../models/log_entry.dart';
import '../../models/mirror_status.dart';
import '../../services/native_bridge.dart';
import '../../services/permissions_service.dart';
import '../../services/usb_channel.dart';

/// Owns everything the mirror screen needs to know.
///
/// All of this used to live inside one `State` class next to the widget tree,
/// so any change to a timer or a permission flow meant editing the same file
/// as the layout. Keeping it here also makes the ordering constraints
/// testable, which is where the real bugs were.
class MirrorController extends ChangeNotifier {
  MirrorController({
    NativeBridge? native,
    UsbChannel? channel,
    PermissionsService? permissions,
  })  : _native = native ?? NativeBridge(),
        _channel = channel ?? UsbChannel(),
        _permissions = permissions ?? const PermissionsService();

  final NativeBridge _native;
  final UsbChannel _channel;
  final PermissionsService _permissions;

  /// How often to check for commands from the desktop and for a dropped link.
  static const _commandPollInterval = Duration(milliseconds: 500);

  /// How often to refresh the throughput figures.
  static const _metricsInterval = Duration(seconds: 1);

  /// Entries kept before the oldest are dropped.
  static const _maxLogEntries = 200;

  MirrorPhase _phase = MirrorPhase.idle;
  String? _statusOverride;
  bool _nativeReady = false;
  MirrorMetrics _metrics = MirrorMetrics.zero;
  Duration _uptime = Duration.zero;
  final List<LogEntry> _logs = [];

  Timer? _commandTimer;
  Timer? _metricsTimer;
  Timer? _uptimeTimer;
  bool _disposed = false;

  MirrorPhase get phase => _phase;
  bool get isStreaming => _phase == MirrorPhase.streaming;
  bool get nativeReady => _nativeReady;
  MirrorMetrics get metrics => _metrics;
  Duration get uptime => _uptime;
  List<LogEntry> get logs => List.unmodifiable(_logs);

  /// A message that replaces the phase's default wording, if there is one.
  ///
  /// Returned as a resolver rather than a string because the controller has no
  /// `BuildContext` and must not hold translated text — that would freeze the
  /// message in whatever language was active when it was set.
  String? get statusOverrideKey => _statusOverride;

  /// Starts the app: permissions, USB listening, and the native library.
  ///
  /// The native library is loaded last and awaited, because nothing else can
  /// make progress without it and the failure message should be the last thing
  /// in the log rather than buried above the permission results.
  Future<void> start() async {
    _log(LogCode.appStarted);
    _listenForUsb();
    await _requestPermissions();
    await _loadNative();
    await _recoverPendingAccessory();
  }

  Future<void> _requestPermissions() async {
    _log(await _permissions.requestAudioCapture()
        ? LogCode.audioPermissionGranted
        : LogCode.audioPermissionDenied);

    if (!await _permissions.requestNotifications()) {
      _log(LogCode.notificationPermissionDenied);
    }
  }

  Future<void> _loadNative() async {
    _log(LogCode.nativeLoading);
    try {
      await _native.initialize();
      _nativeReady = true;
      _log(LogCode.nativeReady);
    } catch (error) {
      _log(LogCode.nativeFailed, detail: '$error');
    }
    _notify();
  }

  void _listenForUsb() {
    _channel.listen(UsbChannelHandlers(
      onAttached: _onAttached,
      onDetached: _onDetached,
      onProjectionStarted: _onCaptureApproved,
      onProjectionDenied: _onCaptureDenied,
    ));
    _log(LogCode.usbListening);
  }

  /// Picks up a cable that was plugged in before Dart was ready to hear about
  /// it — which is the usual case, since the attach event is what launched the
  /// app.
  Future<void> _recoverPendingAccessory() async {
    try {
      final fd = await _channel.pendingAccessory();
      if (fd != null && fd >= 0) {
        _log(LogCode.usbRecovered);
        _onAttached(fd);
      }
    } catch (error) {
      _log(LogCode.usbQueryFailed, detail: '$error');
    }
  }

  void _onAttached(int fd) {
    _log(LogCode.usbAttached);
    _setPhase(MirrorPhase.connecting);

    if (!_nativeReady) {
      _setPhase(MirrorPhase.error);
      return;
    }

    _log(LogCode.pipelineStarting);
    unawaited(_openPipeline(fd));
  }

  Future<void> _openPipeline(int fd) async {
    try {
      await _native.startStreaming(fd);
      _log(LogCode.pipelineStarted);
      _setPhase(MirrorPhase.linked);
      _startCommandPolling();
    } catch (error) {
      _log(LogCode.pipelineFailed, detail: '$error');
      _setPhase(MirrorPhase.error);
    }
  }

  void _startCommandPolling() {
    _commandTimer?.cancel();
    _commandTimer = Timer.periodic(_commandPollInterval, (_) => _poll());
  }

  Future<void> _poll() async {
    if (!_nativeReady) return;

    final command = _native.pollCommand();
    if (command != null) {
      await _handleCommand(command);
      return;
    }

    // The Android detach broadcast is not always delivered — pulling the cable
    // mid-transfer routinely skips it — so the native link state is the
    // authority on whether the connection is still up.
    if (_phase == MirrorPhase.idle || _phase == MirrorPhase.error) return;
    final state = _native.connectionState();
    if (state == 'idle' || state.startsWith('error:')) {
      _log(LogCode.connectionLost, detail: state);
      await _stopService();
      _onDetached();
    }
  }

  Future<void> _handleCommand(Map<String, dynamic> command) async {
    _log(LogCode.configReceived);
    switch (command['command']) {
      case 'start':
        await _requestCapture(command);
      case 'stop':
        _log(LogCode.stopRequested);
        await _stopService();
        _stopStreamingTimers();
        _metrics = MirrorMetrics.zero;
        _statusOverride = 'ready';
        _setPhase(MirrorPhase.linked);
        _log(LogCode.stopped);
    }
  }

  Future<void> _requestCapture(Map<String, dynamic> command) async {
    _log(LogCode.captureRequested);
    try {
      await _channel.setConfig({
        'resolution': command['resolution'] ?? '1080p',
        'bitrate': command['bitrate'] ?? '8 Mbps',
        'fps': command['fps'] ?? '60',
      });
      await _channel.requestScreenCapture();

      // Deliberately not `streaming` yet. This only opened the system consent
      // dialog; the user has not answered it. Claiming to stream here — which
      // an earlier version did — was wrong whether or not they went on to
      // approve.
      _statusOverride = 'awaitingApproval';
      _setPhase(MirrorPhase.connecting);
    } catch (error) {
      _log(LogCode.captureRequestFailed, detail: '$error');
    }
  }

  void _onCaptureApproved() {
    _log(LogCode.captureApproved);
    _statusOverride = null;
    _setPhase(MirrorPhase.streaming);

    _uptime = Duration.zero;
    _uptimeTimer?.cancel();
    _uptimeTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      _uptime += const Duration(seconds: 1);
      _notify();
    });

    _metricsTimer?.cancel();
    _metricsTimer = Timer.periodic(_metricsInterval, (_) => _refreshMetrics());

    _log(LogCode.streaming);
  }

  void _onCaptureDenied() {
    _log(LogCode.captureDenied);
    _statusOverride = 'captureDeclined';
    _setPhase(MirrorPhase.linked);
  }

  Future<void> _refreshMetrics() async {
    if (_phase != MirrorPhase.streaming) return;
    _metrics = await _native.metrics();
    _notify();
  }

  void _onDetached() {
    _log(LogCode.usbDetached);
    _commandTimer?.cancel();
    _stopStreamingTimers();
    _metrics = MirrorMetrics.zero;
    _uptime = Duration.zero;
    _statusOverride = null;
    _setPhase(MirrorPhase.idle);
    _log(LogCode.pipelineStopped);
  }

  void _stopStreamingTimers() {
    _metricsTimer?.cancel();
    _uptimeTimer?.cancel();
    _metricsTimer = null;
    _uptimeTimer = null;
  }

  Future<void> _stopService() async {
    try {
      await _channel.stopService();
    } catch (_) {
      // Already stopped, or the service was never started. Either way there is
      // nothing for the user to do about it.
    }
  }

  void _setPhase(MirrorPhase phase) {
    _phase = phase;
    _notify();
  }

  void _log(LogCode code, {String? detail}) {
    _logs.add(LogEntry(time: DateTime.now(), code: code, detail: detail));
    if (_logs.length > _maxLogEntries) {
      _logs.removeRange(0, _logs.length - _maxLogEntries);
    }
    _notify();
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  @override
  void dispose() {
    _disposed = true;
    _commandTimer?.cancel();
    _stopStreamingTimers();
    super.dispose();
  }
}
