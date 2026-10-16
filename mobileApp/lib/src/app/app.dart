import 'package:flutter/material.dart';
import 'package:flutter_localizations/flutter_localizations.dart';

import '../features/mirror/mirror_screen.dart';
import '../l10n/generated/app_localizations.dart';
import 'theme.dart';

/// Root widget: theme, localisation, and the one screen.
///
/// The locale is not pinned, so the app follows the phone's language setting.
/// Adding a language is a new `app_<code>.arb` beside `app_en.arb`;
/// `supportedLocales` comes from the generated delegate, so it picks the new
/// file up without a change here.
class MirrorApp extends StatelessWidget {
  const MirrorApp({super.key});

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      onGenerateTitle: (context) => AppLocalizations.of(context).appName,
      debugShowCheckedModeBanner: false,
      theme: MirrorTheme.build(),
      localizationsDelegates: const [
        AppLocalizations.delegate,
        GlobalMaterialLocalizations.delegate,
        GlobalWidgetsLocalizations.delegate,
        GlobalCupertinoLocalizations.delegate,
      ],
      supportedLocales: AppLocalizations.supportedLocales,
      home: const MirrorScreen(),
    );
  }
}
