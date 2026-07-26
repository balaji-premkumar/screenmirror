import 'package:flutter/material.dart';

/// Colours and text styles used across the app.
///
/// Gathered here so a restyle is one file rather than a search for hex
/// literals, and so the severity colours the log panel uses are the same ones
/// the status card uses.
abstract final class MirrorTheme {
  static const background = Color(0xFF050505);
  static const surface = Color(0xFF0A0A0A);
  static const panel = Color(0xFF080808);

  static const accent = Colors.orange;
  static const success = Colors.green;
  static const warning = Colors.yellow;
  static const danger = Colors.red;
  static const info = Colors.blue;

  /// A hairline that reads as a division without drawing attention.
  static Color divider(BuildContext _) => Colors.white.withValues(alpha: 0.04);

  static ThemeData build() => ThemeData(
        brightness: Brightness.dark,
        scaffoldBackgroundColor: background,
        useMaterial3: true,
        colorScheme: ColorScheme.fromSeed(
          seedColor: accent,
          brightness: Brightness.dark,
        ),
      );

  /// The wide-tracked uppercase style used for headings and labels.
  static TextStyle label({
    required Color color,
    double size = 9,
    double spacing = 3,
    FontWeight weight = FontWeight.w900,
  }) =>
      TextStyle(
        color: color,
        fontSize: size,
        fontWeight: weight,
        letterSpacing: spacing,
      );

  static const monospace = TextStyle(fontFamily: 'monospace');
}
