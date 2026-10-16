import 'package:permission_handler/permission_handler.dart';

/// The runtime permissions this app asks for.
///
/// Wrapped rather than called directly so the controller depends on an object
/// it is given, not on a package-level singleton that reaches straight for a
/// platform channel. That is what lets the connection logic be tested without
/// a device attached.
class PermissionsService {
  const PermissionsService();

  /// Permission to capture audio.
  ///
  /// The app records device output through `AudioPlaybackCapture`, not the
  /// microphone, but Android gates both behind `RECORD_AUDIO`. Refusing it
  /// costs sound, not the whole session.
  Future<bool> requestAudioCapture() async =>
      (await Permission.microphone.request()).isGranted;

  /// Permission to post notifications.
  ///
  /// Android 13+ hides the foreground-service notification without it, and
  /// that notification carries the only Stop control the user has once they
  /// leave the app.
  Future<bool> requestNotifications() async =>
      (await Permission.notification.request()).isGranted;
}
