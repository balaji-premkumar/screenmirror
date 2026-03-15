import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:permission_handler/permission_handler.dart';
import 'dart:async';
import 'dart:convert';
import 'src/rust/frb_generated.dart';
import 'src/rust/api.dart' as rust_api;

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
  SystemChrome.setSystemUIOverlayStyle(const SystemUiOverlayStyle(
    statusBarColor: Colors.transparent,
    statusBarIconBrightness: Brightness.light,
    systemNavigationBarColor: Color(0xFF050505),
  ));
  runApp(const MirrorCompanionApp());
}

class MirrorCompanionApp extends StatelessWidget {
  const MirrorCompanionApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'Mirror Companion',
      debugShowCheckedModeBanner: false,
      theme: ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: const Color(0xFF050505),
        useMaterial3: true,
      ),
      home: const CompanionDashboard(),
    );
  }
}

class CompanionDashboard extends StatefulWidget {
  const CompanionDashboard({super.key});

  @override
  State<CompanionDashboard> createState() => _CompanionDashboardState();
}

class _CompanionDashboardState extends State<CompanionDashboard>
    with TickerProviderStateMixin {
  // ── State ──
  String _connState = 'idle'; // idle | connecting | streaming | error
  String _statusMsg = 'Waiting for USB connection...';
  bool _rustReady = false;

  // Metrics
  double _throughput = 0;
  int _latencyMs = 0;
  double _fps = 0;

  // Logs
  final List<_Log> _logs = [];
  final ScrollController _logScroll = ScrollController();

  // Timers
  Timer? _metricsTimer;
  Timer? _uptimeTimer;
  Timer? _configPollTimer;
  Duration _uptime = Duration.zero;

  // Animations
  late AnimationController _pulse;
  late AnimationController _glow;

  // USB channel
  static const _ch = MethodChannel('com.mirror.stream/usb');

  @override
  void initState() {
    super.initState();
    _pulse = AnimationController(vsync: this, duration: const Duration(milliseconds: 1200))..repeat(reverse: true);
    _glow = AnimationController(vsync: this, duration: const Duration(seconds: 3))..repeat(reverse: true);
    _log('SYSTEM', 'Mirror Companion v1.0 started');
    _requestPermissions();
    _setupUsb();
    _initRust();
    _checkInitialAccessory();
  }

  Future<void> _checkInitialAccessory() async {
    // Small delay to let Rust initialize first if possible
    await Future.delayed(const Duration(milliseconds: 500));
    try {
      final int? fd = await _ch.invokeMethod<int>('getInitialAccessory');
      if (fd != null && fd >= 0) {
        _log('USB', 'Recovered existing accessory — FD=$fd');
        _onConnected(fd);
      }
    } catch (e) {
      _log('WARN', 'Initial accessory check failed: $e');
    }
  }

  Future<void> _requestPermissions() async {
    final status = await Permission.microphone.request();
    if (status.isGranted) {
      _log('SYSTEM', 'Microphone permission granted');
    } else {
      _log('WARN', 'Microphone permission denied — audio will not work');
    }
  }

  @override
  void dispose() {
    _metricsTimer?.cancel();
    _uptimeTimer?.cancel();
    _pulse.dispose();
    _glow.dispose();
    _logScroll.dispose();
    super.dispose();
  }

  // ── Logging ─────────────────────────────────────────────────
  void _log(String tag, String msg) {
    final now = DateTime.now();
    final ts = '${now.hour.toString().padLeft(2, '0')}:${now.minute.toString().padLeft(2, '0')}:${now.second.toString().padLeft(2, '0')}';
    if (!mounted) return;
    setState(() {
      _logs.add(_Log(ts: ts, tag: tag, msg: msg));
      if (_logs.length > 200) _logs.removeRange(0, _logs.length - 200);
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_logScroll.hasClients) {
        _logScroll.animateTo(_logScroll.position.maxScrollExtent,
            duration: const Duration(milliseconds: 100), curve: Curves.easeOut);
      }
    });
  }

  // ── USB Listener ────────────────────────────────────────────
  void _setupUsb() async {
    _ch.setMethodCallHandler((call) async {
      if (call.method == 'onUsbAttached') {
        final int fd = call.arguments as int;
        _log('USB', 'Accessory attached — FD=$fd');
        _onConnected(fd);
      } else if (call.method == 'onUsbDetached') {
        _log('USB', 'Accessory detached');
        _onDisconnected();
      }
      return null;
    });
    _log('USB', 'Listening for accessory events...');

    // Attempt to recover pending accessory that fired before Dart compiled/was ready
    try {
      final int? fd = await _ch.invokeMethod<int>('getInitialAccessory');
      if (fd != null && fd >= 0) {
        _log('USB', 'Recovered pending accessory — FD=$fd');
        _onConnected(fd);
      }
    } catch (e) {
      _log('WARN', 'Could not query initial accessory: $e');
    }
  }

  // ── Rust Init (non-blocking) ────────────────────────────────
  Future<void> _initRust() async {
    _log('RUST', 'Loading native library...');
    try {
      await RustLib.init().timeout(const Duration(seconds: 8), onTimeout: () {
        throw TimeoutException('RustLib.init() exceeded 8s');
      });
      if (!mounted) return;
      setState(() => _rustReady = true);
      _log('RUST', 'Native library ready ✓');
    } catch (e) {
      _log('WARN', 'Rust unavailable: $e');
      _log('SYSTEM', 'Running in JNI-only mode — streaming still works');
    }
  }

  // ── Connection Handlers ─────────────────────────────────────
  void _onConnected(int fd) {
    setState(() {
      _connState = 'connecting';
      _statusMsg = 'Establishing pipeline...';
    });

    _log('PIPE', 'Initializing video encoder...');

    if (_rustReady) {
      _startRustPipeline(fd);
    } else {
      // JNI path: MirrorForegroundService pushes directly
      _log('PIPE', 'Using JNI pipeline (Rust unavailable)');
      _setStreaming();
    }

    _configPollTimer?.cancel();
    _configPollTimer = Timer.periodic(const Duration(milliseconds: 500), (_) async {
      try {
        // 1. Poll for commands from the desktop
        final configStr = rust_api.pollConfig();
        if (configStr != null && configStr.isNotEmpty) {
          _log('CONFIG', 'Received config from desktop: $configStr');
          final config = jsonDecode(configStr);
          if (config["command"] == "start") {
             _requestMediaProjection(config);
          } else if (config["command"] == "stop") {
             _log('CONTROL', 'Desktop requested stop');
             await _ch.invokeMethod('stopService');
             _onDisconnected();
             return;
          }
        }

        // 2. Poll the Rust USB connection state to detect disconnection
        final rustState = rust_api.getConnectionState();
        if (_connState == 'streaming' || _connState == 'connecting') {
          if (rustState == 'idle' || rustState.startsWith('error:')) {
            _log('USB', 'Connection lost (Rust state: $rustState)');
            try { await _ch.invokeMethod('stopService'); } catch (_) {}
            _onDisconnected();
          }
        }
      } catch (e) {
        // parsing error or FFI not ready
      }
    });
  }

  void _onDisconnected() {
    _uptimeTimer?.cancel();
    _metricsTimer?.cancel();
    _configPollTimer?.cancel();
    setState(() {
      _connState = 'idle';
      _statusMsg = 'Waiting for USB connection...';
      _uptime = Duration.zero;
      _throughput = 0;
      _fps = 0;
      _latencyMs = 0;
    });
    _log('SYSTEM', 'Pipeline stopped — ready for reconnection');
  }

  Future<void> _startRustPipeline(int fd) async {
    try {
      _log('PIPE', 'Starting Rust USB streaming on FD=$fd');
      final result = await rust_api.startUsbStreaming(fd: fd);
      _log('SUCCESS', result);
      _setStreaming();
    } catch (e) {
      _log('ERROR', 'Rust pipeline failed: $e');
      _log('PIPE', 'Falling back to JNI mode');
      _setStreaming();
    }
  }

  /// Request screen capture permission from the system.
  /// On approval, MainActivity starts MirrorForegroundService which
  /// configures MediaCodec (H.265 encoder) and begins pushing encoded
  /// frames through the JNI bridge → Rust Muxer → USB write loop.
  Future<void> _requestMediaProjection(Map<dynamic, dynamic>? config) async {
    try {
      _log('CAPTURE', 'Requesting screen capture permission...');
      if (config != null) {
         await _ch.invokeMethod('setConfig', config);
      }
      await _ch.invokeMethod('requestMediaProjection');
      _log('SUCCESS', 'MediaProjection request sent to system');
    } catch (e) {
      _log('ERROR', 'MediaProjection request failed: $e');
    }
  }

  void _setStreaming() {
    setState(() {
      _connState = 'streaming';
      _statusMsg = 'Streaming to PC';
    });
    _uptimeTimer?.cancel();
    _uptime = Duration.zero;
    _uptimeTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() => _uptime += const Duration(seconds: 1));
    });
    _startMetrics();
    _log('SUCCESS', 'Video + audio flowing to desktop ✓');
  }

  void _startMetrics() {
    _metricsTimer?.cancel();
    if (!_rustReady) return;
    _metricsTimer = Timer.periodic(const Duration(seconds: 1), (_) async {
      if (_connState != 'streaming') return;
      try {
        final json = await rust_api.getMobileMetrics();
        final m = jsonDecode(json);
        if (mounted) {
          setState(() {
            _throughput = (m['throughput_mbps'] ?? 0).toDouble();
            _latencyMs = (m['encoding_latency_ms'] ?? 0).toInt();
            _fps = (m['fps_actual'] ?? 0).toDouble();
          });
        }
      } catch (_) {}
    });
  }

  // ── UI ──────────────────────────────────────────────────────
  @override
  Widget build(BuildContext context) {
    final streaming = _connState == 'streaming';
    final connecting = _connState == 'connecting';
    final error = _connState == 'error';

    return Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            _header(streaming),
            _statusCard(streaming, connecting, error),
            if (streaming) _metrics(),
            Padding(
              padding: const EdgeInsets.symmetric(horizontal: 24),
              child: Divider(color: Colors.white.withValues(alpha: 0.04), height: 1),
            ),
            Expanded(child: _logPanel()),
            _footer(streaming),
          ],
        ),
      ),
    );
  }

  // ── Header ──────────────────────────────────────────────────
  Widget _header(bool streaming) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 20, 24, 0),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            const Text('MIRROR',
              style: TextStyle(color: Colors.orange, fontSize: 20, fontWeight: FontWeight.w900, letterSpacing: 6)),
            const SizedBox(height: 2),
            Text('COMPANION',
              style: TextStyle(color: Colors.white.withValues(alpha: 0.25), fontSize: 10, fontWeight: FontWeight.w700, letterSpacing: 8)),
          ]),
          Row(children: [
            // Rust / JNI badge
            Container(
              padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
              decoration: BoxDecoration(
                color: (_rustReady ? Colors.green : Colors.yellow).withValues(alpha: 0.08),
                borderRadius: BorderRadius.circular(6),
                border: Border.all(color: (_rustReady ? Colors.green : Colors.yellow).withValues(alpha: 0.15)),
              ),
              child: Text(_rustReady ? 'NATIVE' : 'JNI',
                style: TextStyle(color: _rustReady ? Colors.green : Colors.yellow, fontSize: 8, fontWeight: FontWeight.w900, letterSpacing: 2)),
            ),
            const SizedBox(width: 10),
            FadeTransition(
              opacity: _pulse,
              child: Container(width: 8, height: 8, decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: streaming ? Colors.green : Colors.red,
                boxShadow: [BoxShadow(color: (streaming ? Colors.green : Colors.red).withValues(alpha: 0.5), blurRadius: 6, spreadRadius: 2)],
              )),
            ),
          ]),
        ],
      ),
    );
  }

  // ── Status Card ─────────────────────────────────────────────
  Widget _statusCard(bool streaming, bool connecting, bool error) {
    Color c = Colors.white.withValues(alpha: 0.3);
    if (streaming) c = Colors.green;
    if (connecting) c = Colors.orange;
    if (error) c = Colors.red;

    return AnimatedBuilder(
      animation: _glow,
      builder: (ctx, _) {
        final g = streaming ? 0.02 + _glow.value * 0.04 : 0.0;
        return Container(
          margin: const EdgeInsets.fromLTRB(24, 24, 24, 16),
          padding: const EdgeInsets.all(24),
          decoration: BoxDecoration(
            color: Color.lerp(const Color(0xFF0a0a0a), c, g),
            borderRadius: BorderRadius.circular(20),
            border: Border.all(color: c.withValues(alpha: streaming ? 0.15 : 0.06)),
          ),
          child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
            Row(children: [
              Container(width: 6, height: 6, decoration: BoxDecoration(shape: BoxShape.circle, color: c)),
              const SizedBox(width: 10),
              Expanded(child: Text(_statusMsg.toUpperCase(),
                style: TextStyle(color: c, fontSize: 13, fontWeight: FontWeight.w900, letterSpacing: 1))),
            ]),
            if (streaming) ...[
              const SizedBox(height: 12),
              Text(_fmtUptime(_uptime),
                style: TextStyle(color: Colors.white.withValues(alpha: 0.18), fontSize: 11, fontWeight: FontWeight.w600, letterSpacing: 3)),
            ],
            if (connecting) ...[
              const SizedBox(height: 16),
              const LinearProgressIndicator(color: Colors.orange, backgroundColor: Colors.transparent),
            ],
          ]),
        );
      },
    );
  }

  // ── Metrics Row ─────────────────────────────────────────────
  Widget _metrics() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
      child: Row(children: [
        _chip('THROUGHPUT', '${_throughput.toStringAsFixed(1)} Mbps', Colors.blue),
        const SizedBox(width: 8),
        _chip('LATENCY', '${_latencyMs}ms', Colors.green),
        const SizedBox(width: 8),
        _chip('FPS', _fps.toStringAsFixed(0), Colors.orange),
      ]),
    );
  }

  Widget _chip(String label, String value, Color c) {
    return Expanded(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        decoration: BoxDecoration(
          color: c.withValues(alpha: 0.05),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: c.withValues(alpha: 0.08)),
        ),
        child: Column(crossAxisAlignment: CrossAxisAlignment.start, children: [
          Text(label, style: TextStyle(color: c.withValues(alpha: 0.45), fontSize: 7, fontWeight: FontWeight.w900, letterSpacing: 1.5)),
          const SizedBox(height: 4),
          Text(value, style: TextStyle(color: c, fontSize: 14, fontWeight: FontWeight.w900)),
        ]),
      ),
    );
  }

  // ── Log Panel ───────────────────────────────────────────────
  Widget _logPanel() {
    return Container(
      margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      decoration: BoxDecoration(
        color: const Color(0xFF080808),
        borderRadius: const BorderRadius.vertical(top: Radius.circular(16)),
        border: Border.all(color: Colors.white.withValues(alpha: 0.03)),
      ),
      child: Column(children: [
        Padding(
          padding: const EdgeInsets.fromLTRB(16, 12, 16, 8),
          child: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
            Text('DIAGNOSTIC LOG',
              style: TextStyle(color: Colors.white.withValues(alpha: 0.2), fontSize: 9, fontWeight: FontWeight.w900, letterSpacing: 3)),
            Text('${_logs.length}',
              style: TextStyle(color: Colors.white.withValues(alpha: 0.12), fontSize: 9)),
          ]),
        ),
        Divider(height: 1, color: Colors.white.withValues(alpha: 0.03)),
        Expanded(
          child: _logs.isEmpty
            ? Center(child: Text('No log entries yet...',
                style: TextStyle(color: Colors.white.withValues(alpha: 0.1), fontSize: 11)))
            : ListView.builder(
                controller: _logScroll,
                padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                itemCount: _logs.length,
                itemBuilder: (_, i) {
                  final l = _logs[i];
                  return Padding(
                    padding: const EdgeInsets.symmetric(vertical: 2),
                    child: Row(crossAxisAlignment: CrossAxisAlignment.start, children: [
                      SizedBox(width: 50, child: Text(l.ts,
                        style: TextStyle(color: Colors.white.withValues(alpha: 0.12), fontSize: 9, fontFamily: 'monospace'))),
                      Container(
                        padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
                        margin: const EdgeInsets.only(right: 8),
                        decoration: BoxDecoration(
                          color: _tagClr(l.tag).withValues(alpha: 0.08),
                          borderRadius: BorderRadius.circular(3),
                        ),
                        child: Text(l.tag,
                          style: TextStyle(color: _tagClr(l.tag), fontSize: 8, fontWeight: FontWeight.w900, letterSpacing: 0.5)),
                      ),
                      Expanded(child: Text(l.msg,
                        style: TextStyle(color: Colors.white.withValues(alpha: 0.5), fontSize: 10, height: 1.4, fontFamily: 'monospace'))),
                    ]),
                  );
                },
              ),
        ),
      ]),
    );
  }

  // ── Footer ──────────────────────────────────────────────────
  Widget _footer(bool streaming) {
    return Container(
      padding: const EdgeInsets.fromLTRB(24, 10, 24, 14),
      child: Row(mainAxisAlignment: MainAxisAlignment.spaceBetween, children: [
        Text(streaming ? 'LINKED TO DESKTOP' : 'PLUG USB TO START',
          style: TextStyle(color: Colors.white.withValues(alpha: 0.12), fontSize: 9, fontWeight: FontWeight.w800, letterSpacing: 3)),
        Text('AUTO-CONNECT',
          style: TextStyle(color: Colors.orange.withValues(alpha: 0.25), fontSize: 9, fontWeight: FontWeight.w800, letterSpacing: 2)),
      ]),
    );
  }

  // ── Helpers ─────────────────────────────────────────────────
  Color _tagClr(String tag) => switch (tag) {
    'ERROR' => Colors.red,
    'WARN' => Colors.yellow,
    'SUCCESS' => Colors.green,
    'USB' => Colors.blue,
    'PIPE' || 'CAPTURE' => Colors.purple,
    'RUST' => Colors.teal,
    _ => Colors.grey,
  };

  String _fmtUptime(Duration d) =>
    '${d.inHours.toString().padLeft(2, '0')}:${(d.inMinutes % 60).toString().padLeft(2, '0')}:${(d.inSeconds % 60).toString().padLeft(2, '0')}';
}

class _Log {
  final String ts, tag, msg;
  const _Log({required this.ts, required this.tag, required this.msg});
}
