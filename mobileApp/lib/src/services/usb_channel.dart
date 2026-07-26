import 'package:flutter/services.dart';

/// Events the Android side pushes up to Dart.
///
/// [onProjectionStarted] is the one that matters: it fires when the foreground
/// service is actually running with an approved capture token, which is the
/// only moment it is true to say anything is being streamed.
class UsbChannelHandlers {
  const UsbChannelHandlers({
    required this.onAttached,
    required this.onDetached,
    required this.onProjectionStarted,
    required this.onProjectionDenied,
  });

  final void Function(int fd) onAttached;
  final VoidCallback onDetached;
  final VoidCallback onProjectionStarted;
  final VoidCallback onProjectionDenied;
}

/// Wraps the platform channel shared with `MainActivity.kt`.
///
/// Kept apart from the UI so a widget never constructs a [MethodChannel] or
/// spells a method name; both sides of this boundary are strings, and a typo
/// in one of them fails at runtime with no warning.
class UsbChannel {
  UsbChannel([MethodChannel? channel])
      : _channel = channel ?? const MethodChannel(channelName);

  /// Must match `CHANNEL` in `MainActivity.kt`.
  static const channelName = 'com.mirror.stream/usb';

  final MethodChannel _channel;

  /// Routes incoming calls from Android to [handlers].
  void listen(UsbChannelHandlers handlers) {
    _channel.setMethodCallHandler((call) async {
      switch (call.method) {
        case 'onUsbAttached':
          handlers.onAttached(call.arguments as int);
        case 'onUsbDetached':
          handlers.onDetached();
        case 'onProjectionStarted':
          handlers.onProjectionStarted();
        case 'onProjectionDenied':
          handlers.onProjectionDenied();
      }
      return null;
    });
  }

  /// The file descriptor of an accessory attached before Dart was ready.
  ///
  /// Android launches the app *because* the cable was plugged in, so the
  /// attach event routinely fires before the Dart isolate exists to hear it.
  /// Without this poll, plugging in a cold-started app connected nothing.
  Future<int?> pendingAccessory() => _channel.invokeMethod<int>('getInitialAccessory');

  /// Sends capture settings for the service to apply on the next start.
  Future<void> setConfig(Map<String, Object?> config) =>
      _channel.invokeMethod<void>('setConfig', config);

  /// Opens the system screen-capture consent dialog.
  ///
  /// Returning does not mean capture started — only that the dialog was shown.
  /// The answer arrives as `onProjectionStarted` or `onProjectionDenied`.
  Future<void> requestScreenCapture() =>
      _channel.invokeMethod<void>('requestMediaProjection');

  /// Stops the foreground service, ending capture.
  Future<void> stopService() => _channel.invokeMethod<void>('stopService');
}
