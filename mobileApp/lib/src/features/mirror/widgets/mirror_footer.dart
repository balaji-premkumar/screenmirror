import 'package:flutter/material.dart';

import '../../../app/theme.dart';
import '../../../l10n/generated/app_localizations.dart';

/// The one-line hint along the bottom of the screen.
class MirrorFooter extends StatelessWidget {
  const MirrorFooter({super.key, required this.isStreaming});

  final bool isStreaming;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);

    return Container(
      padding: const EdgeInsets.fromLTRB(24, 10, 24, 14),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Flexible(
            child: Text(
              (isStreaming ? l10n.footerLinked : l10n.footerPlugIn).toUpperCase(),
              style: MirrorTheme.label(
                color: Colors.white.withValues(alpha: 0.12),
                weight: FontWeight.w800,
              ),
            ),
          ),
          const SizedBox(width: 12),
          Text(
            l10n.footerAutoConnect.toUpperCase(),
            style: MirrorTheme.label(
              color: MirrorTheme.accent.withValues(alpha: 0.25),
              spacing: 2,
              weight: FontWeight.w800,
            ),
          ),
        ],
      ),
    );
  }
}
