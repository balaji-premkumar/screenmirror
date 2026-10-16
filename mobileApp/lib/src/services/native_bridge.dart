import 'dart:async';
import 'dart:convert';

import '../models/mirror_status.dart';
import '../rust/api.dart' as rust_api;
import '../rust/frb_generated.dart';

/// The Rust library, behind Dart types.
///
/// The generated bindings in `lib/src/rust/` return JSON strings and raw
/// values; decoding them at every call site meant `jsonDecode` and a bare map
/// lookup scattered through the UI. This is the one place that knows the
/// native side's shapes.
class NativeBridge {
  bool _ready = false;

  /// Whether [initialize] has succeeded. Nothing else here works until it has.
  bool get isReady => _ready;

  /// How long to wait for the native library before giving up.
  ///
  /// Loading it involves `dlopen` of a several-megabyte library on a cold
  /// filesystem cache; eight seconds is slow enough to be safe on a low-end
  /// phone and short enough that a genuinely missing library does not hang the
  /// UI indefinitely.
  static const _initTimeout = Duration(seconds: 8);

  /// Loads the native library.
  ///
  /// Throws if it cannot be loaded. The JNI bridge lives in the same shared
  /// object as these bindings, so a failure here means streaming cannot work
  /// at all — it is not something to degrade past quietly.
  Future<void> initialize() async {
    await RustLib.init().timeout(
      _initTimeout,
      onTimeout: () => throw TimeoutException(
        'RustLib.init() did not finish within $_initTimeout',
      ),
    );
    _ready = true;
  }

  /// Starts the USB streaming pipeline on an accessory file descriptor.
  Future<String> startStreaming(int fd) => rust_api.startUsbStreaming(fd: fd);

  /// The next queued command from the desktop, or `null` if there is none.
  Map<String, dynamic>? pollCommand() {
    if (!_ready) return null;
    final raw = rust_api.pollConfig();
    if (raw == null || raw.isEmpty) return null;
    try {
      final decoded = jsonDecode(raw);
      return decoded is Map<String, dynamic> ? decoded : null;
    } on FormatException {
      // A truncated USB read can leave a partial JSON object. Dropping it is
      // right: the desktop resends, and the alternative is throwing on the
      // polling timer twice a second.
      return null;
    }
  }

  /// The native view of the USB link: `idle`, `connected`, `streaming`, or
  /// `error: ...`.
  ///
  /// Polled because the Android detach broadcast is not always delivered — if
  /// the cable is pulled mid-transfer, this is what notices.
  String connectionState() => _ready ? rust_api.getConnectionState() : 'idle';

  /// Current throughput figures, or zeroes if they cannot be read.
  Future<MirrorMetrics> metrics() async {
    if (!_ready) return MirrorMetrics.zero;
    try {
      final decoded = jsonDecode(await rust_api.getMobileMetrics());
      if (decoded is! Map<String, dynamic>) return MirrorMetrics.zero;
      return MirrorMetrics.fromJson(decoded);
    } on FormatException {
      return MirrorMetrics.zero;
    }
  }
}
