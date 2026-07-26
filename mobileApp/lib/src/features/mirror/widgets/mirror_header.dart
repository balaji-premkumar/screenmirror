import 'package:flutter/material.dart';

import '../../../app/theme.dart';
import '../../../l10n/generated/app_localizations.dart';

/// App name, native-library badge, and the connection indicator.
class MirrorHeader extends StatelessWidget {
  const MirrorHeader({
    super.key,
    required this.isStreaming,
    required this.nativeReady,
    required this.pulse,
  });

  final bool isStreaming;
  final bool nativeReady;

  /// Drives the indicator's fade, so the animation lives with the screen that
  /// owns the ticker rather than being restarted by every rebuild here.
  final Animation<double> pulse;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);
    final badgeColour = nativeReady ? MirrorTheme.success : MirrorTheme.warning;
    final dotColour = isStreaming ? MirrorTheme.success : MirrorTheme.danger;

    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 20, 24, 0),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        children: [
          Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(
                l10n.appName.toUpperCase(),
                style: MirrorTheme.label(
                  color: MirrorTheme.accent,
                  size: 20,
                  spacing: 6,
                ),
              ),
              const SizedBox(height: 2),
              Text(
                l10n.appTagline.toUpperCase(),
                style: MirrorTheme.label(
                  color: Colors.white.withValues(alpha: 0.25),
                  size: 10,
                  spacing: 8,
                  weight: FontWeight.w700,
                ),
              ),
            ],
          ),
          Row(
            children: [
              Container(
                padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 4),
                decoration: BoxDecoration(
                  color: badgeColour.withValues(alpha: 0.08),
                  borderRadius: BorderRadius.circular(6),
                  border: Border.all(color: badgeColour.withValues(alpha: 0.15)),
                ),
                child: Text(
                  (nativeReady ? l10n.badgeNative : l10n.badgeLoading).toUpperCase(),
                  style: MirrorTheme.label(color: badgeColour, size: 8, spacing: 2),
                ),
              ),
              const SizedBox(width: 10),
              FadeTransition(
                opacity: pulse,
                child: Container(
                  width: 8,
                  height: 8,
                  decoration: BoxDecoration(
                    shape: BoxShape.circle,
                    color: dotColour,
                    boxShadow: [
                      BoxShadow(
                        color: dotColour.withValues(alpha: 0.5),
                        blurRadius: 6,
                        spreadRadius: 2,
                      ),
                    ],
                  ),
                ),
              ),
            ],
          ),
        ],
      ),
    );
  }
}
