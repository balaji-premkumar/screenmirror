import 'package:flutter_test/flutter_test.dart';
import 'package:stream_mobile_app/src/features/mirror/mirror_controller.dart';
import 'package:stream_mobile_app/src/models/log_entry.dart';
import 'package:stream_mobile_app/src/models/mirror_status.dart';
import 'package:stream_mobile_app/src/services/native_bridge.dart';
import 'package:stream_mobile_app/src/services/permissions_service.dart';
import 'package:stream_mobile_app/src/services/usb_channel.dart';

/// A native bridge that reports whatever the test tells it to.
class _FakeNative extends NativeBridge {
  _FakeNative({this.initFails = false});

  final bool initFails;
  bool _ready = false;
  String state = 'connected';
  Map<String, dynamic>? nextCommand;
  final startedFds = <int>[];

  @override
  bool get isReady => _ready;

  @override
  Future<void> initialize() async {
    if (initFails) throw StateError('no native library');
    _ready = true;
  }

  @override
  Future<String> startStreaming(int fd) async {
    startedFds.add(fd);
    return 'ok';
  }

  @override
  Map<String, dynamic>? pollCommand() {
    final command = nextCommand;
    nextCommand = null;
    return command;
  }

  @override
  String connectionState() => state;

  @override
  Future<MirrorMetrics> metrics() async =>
      const MirrorMetrics(throughputMbps: 10, latencyMs: 5, fps: 60);
}

/// A USB channel that records calls instead of touching the platform.
class _FakeChannel extends UsbChannel {
  _FakeChannel() : super();

  UsbChannelHandlers? handlers;
  int? pending;
  final calls = <String>[];

  @override
  void listen(UsbChannelHandlers handlers) => this.handlers = handlers;

  @override
  Future<int?> pendingAccessory() async => pending;

  @override
  Future<void> setConfig(Map<String, Object?> config) async =>
      calls.add('setConfig:${config['resolution']}');

  @override
  Future<void> requestScreenCapture() async => calls.add('requestScreenCapture');

  @override
  Future<void> stopService() async => calls.add('stopService');
}

/// Grants everything, so permission prompts do not reach a platform channel
/// that does not exist under `flutter test`.
class _FakePermissions implements PermissionsService {
  @override
  Future<bool> requestAudioCapture() async => true;

  @override
  Future<bool> requestNotifications() async => true;
}

void main() {
  late _FakeNative native;
  late _FakeChannel channel;
  late MirrorController controller;

  setUp(() {
    native = _FakeNative();
    channel = _FakeChannel();
    controller = MirrorController(
      native: native,
      channel: channel,
      permissions: _FakePermissions(),
    );
  });

  tearDown(() => controller.dispose());

  test('starts idle and reaches linked once the cable is attached', () async {
    expect(controller.phase, MirrorPhase.idle);

    await controller.start();
    expect(controller.nativeReady, isTrue);

    channel.handlers!.onAttached(42);
    await Future<void>.delayed(Duration.zero);

    expect(native.startedFds, [42]);
    expect(controller.phase, MirrorPhase.linked);
  });

  test('an accessory attached before Dart was ready is recovered', () async {
    // Android launches the app *because* the cable was plugged in, so the
    // attach event routinely fires before this isolate exists. Without the
    // pending-accessory poll, a cold start connected nothing at all.
    channel.pending = 7;
    await controller.start();
    await Future<void>.delayed(Duration.zero);

    expect(native.startedFds, [7]);
    expect(controller.phase, MirrorPhase.linked);
  });

  test('a start command opens the consent dialog but does NOT claim to stream',
      () async {
    // Regression test. An earlier version set the streaming state as soon as
    // it asked for permission, so the phone read "Streaming to PC" while the
    // system dialog was still on screen and might yet be declined.
    await controller.start();
    channel.handlers!.onAttached(1);
    await Future<void>.delayed(Duration.zero);

    native.nextCommand = {'command': 'start', 'resolution': '1080p'};
    await Future<void>.delayed(const Duration(milliseconds: 600));

    expect(channel.calls, contains('requestScreenCapture'));
    expect(controller.phase, isNot(MirrorPhase.streaming));
    expect(controller.isStreaming, isFalse);
  });

  test('only an approved capture moves to streaming', () async {
    await controller.start();
    channel.handlers!.onAttached(1);
    await Future<void>.delayed(Duration.zero);

    channel.handlers!.onProjectionStarted();
    expect(controller.phase, MirrorPhase.streaming);
    expect(controller.isStreaming, isTrue);
  });

  test('a declined capture returns to linked, not to error', () async {
    // Declining is a choice, not a failure: the desktop can ask again.
    await controller.start();
    channel.handlers!.onAttached(1);
    await Future<void>.delayed(Duration.zero);

    channel.handlers!.onProjectionDenied();
    expect(controller.phase, MirrorPhase.linked);
    expect(controller.statusOverrideKey, 'captureDeclined');
  });

  test('detaching clears the metrics and returns to idle', () async {
    await controller.start();
    channel.handlers!.onAttached(1);
    await Future<void>.delayed(Duration.zero);
    channel.handlers!.onProjectionStarted();

    channel.handlers!.onDetached();

    expect(controller.phase, MirrorPhase.idle);
    expect(controller.metrics.throughputMbps, 0);
    expect(controller.uptime, Duration.zero);
  });

  test('a lost native link is noticed even without a detach broadcast',
      () async {
    // Pulling the cable mid-transfer routinely skips the Android broadcast,
    // so the polled native state is what actually catches it.
    await controller.start();
    channel.handlers!.onAttached(1);
    await Future<void>.delayed(Duration.zero);

    native.state = 'error: device gone';
    await Future<void>.delayed(const Duration(milliseconds: 600));

    expect(controller.phase, MirrorPhase.idle);
    expect(channel.calls, contains('stopService'));
  });

  test('a missing native library blocks streaming instead of failing silently',
      () async {
    final broken = MirrorController(
      native: _FakeNative(initFails: true),
      channel: channel,
      permissions: _FakePermissions(),
    );
    addTearDown(broken.dispose);

    await broken.start();
    expect(broken.nativeReady, isFalse);

    channel.handlers!.onAttached(1);
    expect(broken.phase, MirrorPhase.error);
    expect(
      broken.logs.map((entry) => entry.code),
      contains(LogCode.nativeFailed),
    );
  });

  test('the log is bounded so a long session cannot grow without limit',
      () async {
    await controller.start();
    for (var i = 0; i < 500; i++) {
      channel.handlers!.onProjectionDenied();
    }
    expect(controller.logs.length, lessThanOrEqualTo(200));
  });
}
