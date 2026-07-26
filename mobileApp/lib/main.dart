import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import 'src/app/app.dart';
import 'src/app/theme.dart';

/// Entry point.
///
/// Deliberately short: it sets up the system chrome and hands off. Everything
/// else lives under `src/` — `app/` for the shell, `features/mirror/` for the
/// screen and its state, `services/` for the platform channel and the native
/// bridge, `models/` for the shared types, and `l10n/` for the wording.
void main() {
  WidgetsFlutterBinding.ensureInitialized();

  // The layout is built for a single portrait column, and the encoder is
  // configured from the display size at capture time — letting the phone
  // rotate mid-session would change the capture geometry underneath it.
  SystemChrome.setPreferredOrientations([DeviceOrientation.portraitUp]);

  SystemChrome.setSystemUIOverlayStyle(
    const SystemUiOverlayStyle(
      statusBarColor: Colors.transparent,
      statusBarIconBrightness: Brightness.light,
      systemNavigationBarColor: MirrorTheme.background,
    ),
  );

  runApp(const MirrorApp());
}
