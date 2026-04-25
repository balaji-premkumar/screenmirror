import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:permission_handler/permission_handler.dart';
import 'dart:async';
import 'dart:convert';
import 'src/rust/frb_generated.dart';
import 'src/rust/api.dart' as rust_api;

// Design Tokens (Kinetic Precision)
const kPrimaryCyan = Color(0xFF00F0FF);
const kBgVoid = Color(0xFF0A0A0B);
const kSurfaceSlate = Color(0xFF1A1F2C);
const kTextOffWhite = Color(0xFFE0E2E8);
const kTextMuted = Color(0xFF8E9196);

void main() {
  WidgetsFlutterBinding.ensureInitialized();
  SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);
  SystemChrome.setSystemUIOverlayStyle(const SystemUiOverlayStyle(
    statusBarColor: Colors.transparent,
    statusBarIconBrightness: Brightness.light,
    systemNavigationBarColor: kBgVoid,
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
        scaffoldBackgroundColor: kBgVoid,
        useMaterial3: true,
        fontFamily: 'Inter',
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
  String _statusMsg = 'Waiting for USB...';
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

  // USB channel
  static const _ch = MethodChannel('com.mirror.stream/usb');

  @override
  void initState() {
    super.initState();
    _pulse = AnimationController(vsync: this, duration: const Duration(milliseconds: 1500))..repeat(reverse: true);
    _requestPermissions();
    _setupUsb();
    _initRust();
  }

  Future<void> _requestPermissions() async {
    final status = await Permission.microphone.request();
    if (status.isGranted) {
      _log('SYSTEM', 'Audio permissions OK');
    } else {
      _log('WARN', 'Audio denied');
    }
  }

  @override
  void dispose() {
    _metricsTimer?.cancel();
    _uptimeTimer?.cancel();
    _configPollTimer?.cancel();
    _pulse.dispose();
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
      if (_logs.length > 100) _logs.removeRange(0, _logs.length - 100);
    });
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_logScroll.hasClients) {
        _logScroll.jumpTo(_logScroll.position.maxScrollExtent);
      }
    });
  }

  // ── USB Listener ────────────────────────────────────────────
  void _setupUsb() async {
    _ch.setMethodCallHandler((call) async {
      if (call.method == 'onUsbAttached') {
        final int fd = call.arguments as int;
        _log('USB', 'Host attached (FD=$fd)');
        _onConnected(fd);
      } else if (call.method == 'onUsbDetached') {
        _log('USB', 'Host detached');
        _onDisconnected();
      }
      return null;
    });

    try {
      final int? fd = await _ch.invokeMethod<int>('getInitialAccessory');
      if (fd != null && fd >= 0) {
        _log('USB', 'Recovered session');
        _onConnected(fd);
      }
    } catch (_) {}
  }

  Future<void> _initRust() async {
    try {
      await RustLib.init().timeout(const Duration(seconds: 5));
      if (!mounted) return;
      setState(() => _rustReady = true);
      _log('NATIVE', 'Engine initialized ✓');
    } catch (e) {
      _log('NATIVE', 'Running legacy mode');
    }
  }

  // ── Connection Handlers ─────────────────────────────────────
  void _onConnected(int fd) {
    setState(() {
      _connState = 'connecting';
      _statusMsg = 'Linking...';
    });

    if (_rustReady) {
      _startRustPipeline(fd);
    } else {
      _setStreaming();
    }

    _configPollTimer?.cancel();
    _configPollTimer = Timer.periodic(const Duration(milliseconds: 500), (_) async {
      try {
        final configStr = rust_api.pollConfig();
        if (configStr != null && configStr.isNotEmpty) {
          final config = jsonDecode(configStr);
          if (config["command"] == "start") {
             _requestMediaProjection(config);
          } else if (config["command"] == "stop") {
             await _ch.invokeMethod('stopService');
             setState(() {
                _connState = 'connected';
                _statusMsg = 'Ready';
             });
             return;
          }
        }

        final rustState = rust_api.getConnectionState();
        if (_connState == 'streaming' || _connState == 'connecting' || _connState == 'connected') {
          if (rustState == 'idle' || rustState.startsWith('error:')) {
            try { await _ch.invokeMethod('stopService'); } catch (_) {}
            _onDisconnected();
          }
        }
      } catch (_) {}
    });
  }

  void _onDisconnected() {
    _uptimeTimer?.cancel();
    _metricsTimer?.cancel();
    _configPollTimer?.cancel();
    setState(() {
      _connState = 'idle';
      _statusMsg = 'Disconnected';
      _uptime = Duration.zero;
      _throughput = 0;
      _fps = 0;
    });
  }

  Future<void> _startRustPipeline(int fd) async {
    try {
      await rust_api.startUsbStreaming(fd: fd);
      _setStreaming();
    } catch (e) {
      _setStreaming();
    }
  }

  Future<void> _requestMediaProjection(Map<dynamic, dynamic>? config) async {
    try {
      if (config != null) await _ch.invokeMethod('setConfig', config);
      await _ch.invokeMethod('requestMediaProjection');
    } catch (_) {}
  }

  void _setStreaming() {
    setState(() {
      _connState = 'streaming';
      _statusMsg = 'Live Feed';
    });
    _uptimeTimer?.cancel();
    _uptime = Duration.zero;
    _uptimeTimer = Timer.periodic(const Duration(seconds: 1), (_) {
      if (mounted) setState(() => _uptime += const Duration(seconds: 1));
    });
    _startMetrics();
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
            _fps = (m['fps_actual'] ?? 0).toDouble();
          });
        }
      } catch (_) {}
    });
  }

  // ── UI Implementation (Stitch Design) ───────────────────────
  @override
  Widget build(BuildContext context) {
    return Scaffold(
      body: SafeArea(
        child: Column(
          children: [
            _header(),
            _liveIndicator(),
            _metricsGrid(),
            const SizedBox(height: 12),
            Expanded(child: _logPanel()),
            _footer(),
          ],
        ),
      ),
    );
  }

  Widget _header() {
    return Padding(
      padding: const EdgeInsets.all(24),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Row(
            children: [
              Container(
                width: 24, height: 24,
                decoration: BoxDecoration(color: kPrimaryCyan, borderRadius: BorderRadius.circular(4)),
                child: const Center(child: Icon(Icons.bolt, size: 16, color: Colors.black)),
              ),
              const SizedBox(width: 12),
              const Text('MIRROR PRO', style: TextStyle(fontSize: 16, fontWeight: FontWeight.w900, letterSpacing: 2, color: kTextOffWhite)),
            ],
          ),
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
            decoration: BoxDecoration(
              color: Colors.white.withOpacity(0.05),
              borderRadius: BorderRadius.circular(4),
              border: Border.all(color: Colors.white.withOpacity(0.1)),
            ),
            child: Text(_rustReady ? 'NATIVE v2.4' : 'LEGACY', style: const TextStyle(color: kTextMuted, fontSize: 8, fontWeight: FontWeight.bold, letterSpacing: 1)),
          ),
        ],
      ),
    );
  }

  Widget _liveIndicator() {
    final streaming = _connState == 'streaming';
    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 24),
      padding: const EdgeInsets.all(16),
      decoration: BoxDecoration(
        color: kSurfaceSlate,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: streaming ? kPrimaryCyan.withOpacity(0.3) : Colors.white10),
      ),
      child: Row(
        children: [
          FadeTransition(
            opacity: _pulse,
            child: Container(width: 8, height: 8, decoration: BoxDecoration(shape: BoxShape.circle, color: streaming ? kPrimaryCyan : kTextMuted)),
          ),
          const SizedBox(width: 12),
          Text(_statusMsg.toUpperCase(), style: TextStyle(fontSize: 12, fontWeight: FontWeight.bold, color: streaming ? kPrimaryCyan : kTextMuted, letterSpacing: 1)),
          const Spacer(),
          if (streaming) Text(_fmtUptime(_uptime), style: const TextStyle(fontSize: 11, color: kTextMuted, fontFamily: 'monospace')),
        ],
      ),
    );
  }

  Widget _metricsGrid() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 16, 24, 0),
      child: Row(
        children: [
          _metricCard('THROUGHPUT', '${_throughput.toStringAsFixed(1)} Mbps'),
          const SizedBox(width: 12),
          _metricCard('SYNC RATE', '${_fps.toStringAsFixed(0)} FPS'),
        ],
      ),
    );
  }

  Widget _metricCard(String label, String value) {
    return Expanded(
      child: Container(
        padding: const EdgeInsets.all(16),
        decoration: BoxDecoration(
          color: kSurfaceSlate,
          borderRadius: BorderRadius.circular(8),
          border: Border.all(color: Colors.white.withOpacity(0.05)),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(label, style: const TextStyle(color: kTextMuted, fontSize: 8, fontWeight: FontWeight.bold, letterSpacing: 1)),
            const SizedBox(height: 8),
            Text(value, style: const TextStyle(color: kPrimaryCyan, fontSize: 18, fontWeight: FontWeight.w900)),
          ],
        ),
      ),
    );
  }

  Widget _logPanel() {
    return Container(
      margin: const EdgeInsets.symmetric(horizontal: 24, vertical: 12),
      decoration: BoxDecoration(
        color: Colors.black26,
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: Colors.white.withOpacity(0.05)),
      ),
      child: Column(
        children: [
          Container(
            padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
            decoration: BoxDecoration(border: Border(bottom: BorderSide(color: Colors.white.withOpacity(0.05)))),
            child: const Row(
              children: [
                Text('DIAGNOSTIC STREAM', style: TextStyle(color: kTextMuted, fontSize: 9, fontWeight: FontWeight.bold, letterSpacing: 1)),
                Spacer(),
                Icon(Icons.radar, size: 10, color: kTextMuted),
              ],
            ),
          ),
          Expanded(
            child: _logs.isEmpty 
              ? const Center(child: Text('AWAITING DATA...', style: TextStyle(color: Colors.white10, fontSize: 10, letterSpacing: 2)))
              : ListView.builder(
                  controller: _logScroll,
                  padding: const EdgeInsets.all(12),
                  itemCount: _logs.length,
                  itemBuilder: (_, i) {
                    final l = _logs[i];
                    return Padding(
                      padding: const EdgeInsets.only(bottom: 4),
                      child: Row(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          Text(l.ts, style: TextStyle(color: Colors.white.withOpacity(0.1), fontSize: 9, fontFamily: 'monospace')),
                          const SizedBox(width: 8),
                          Container(
                            padding: const EdgeInsets.symmetric(horizontal: 4, vertical: 1),
                            decoration: BoxDecoration(color: _tagClr(l.tag).withOpacity(0.1), borderRadius: BorderRadius.circular(2)),
                            child: Text(l.tag, style: TextStyle(color: _tagClr(l.tag), fontSize: 7, fontWeight: FontWeight.bold)),
                          ),
                          const SizedBox(width: 8),
                          Expanded(child: Text(l.msg, style: TextStyle(color: kTextMuted.withOpacity(0.8), fontSize: 10, height: 1.2))),
                        ],
                      ),
                    );
                  },
                ),
          ),
        ],
      ),
    );
  }

  Widget _footer() {
    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          const Text('KINETIC ENGINE ACTIVE', style: TextStyle(color: Colors.white10, fontSize: 8, fontWeight: FontWeight.bold, letterSpacing: 1)),
          Text(_connState == 'streaming' ? 'ENCRYPTED LINK' : 'READY FOR LINK', style: const TextStyle(color: Colors.white10, fontSize: 8, fontWeight: FontWeight.bold, letterSpacing: 1)),
        ],
      ),
    );
  }

  Color _tagClr(String tag) => switch (tag) {
    'ERROR' => Colors.redAccent,
    'WARN' => Colors.amberAccent,
    'SUCCESS' => kPrimaryCyan,
    _ => kTextMuted,
  };

  String _fmtUptime(Duration d) =>
    '${d.inHours.toString().padLeft(2, '0')}:${(d.inMinutes % 60).toString().padLeft(2, '0')}:${(d.inSeconds % 60).toString().padLeft(2, '0')}';
}

class _Log {
  final String ts, tag, msg;
  const _Log({required this.ts, required this.tag, required this.msg});
}
