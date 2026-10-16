import 'package:flutter/material.dart';

import '../../../app/theme.dart';
import '../../../models/mirror_status.dart';

/// The large card that says what the app is currently doing.
class StatusCard extends StatelessWidget {
  const StatusCard({
    super.key,
    required this.phase,
    required this.message,
    required this.uptime,
    required this.glow,
  });

  final MirrorPhase phase;

  /// Already resolved by the screen, which has the localisations.
  final String message;

  final Duration uptime;

  /// Slow breathing highlight while streaming.
  final Animation<double> glow;

  Color get _colour => switch (phase) {
        MirrorPhase.streaming => MirrorTheme.success,
        MirrorPhase.connecting => MirrorTheme.accent,
        MirrorPhase.error => MirrorTheme.danger,
        MirrorPhase.idle || MirrorPhase.linked => Colors.white.withValues(alpha: 0.3),
      };

  static String _formatUptime(Duration d) =>
      '${d.inHours.toString().padLeft(2, '0')}:'
      '${(d.inMinutes % 60).toString().padLeft(2, '0')}:'
      '${(d.inSeconds % 60).toString().padLeft(2, '0')}';

  @override
  Widget build(BuildContext context) {
    final colour = _colour;
    final isStreaming = phase == MirrorPhase.streaming;

    return AnimatedBuilder(
      animation: glow,
      builder: (context, _) {
        final blend = isStreaming ? 0.02 + glow.value * 0.04 : 0.0;
        return Container(
          margin: const EdgeInsets.fromLTRB(24, 24, 24, 16),
          padding: const EdgeInsets.all(24),
          decoration: BoxDecoration(
            color: Color.lerp(MirrorTheme.surface, colour, blend),
            borderRadius: BorderRadius.circular(20),
            border: Border.all(
              color: colour.withValues(alpha: isStreaming ? 0.15 : 0.06),
            ),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Container(
                    width: 6,
                    height: 6,
                    decoration: BoxDecoration(shape: BoxShape.circle, color: colour),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Text(
                      message.toUpperCase(),
                      style: MirrorTheme.label(color: colour, size: 13, spacing: 1),
                    ),
                  ),
                ],
              ),
              if (isStreaming) ...[
                const SizedBox(height: 12),
                Text(
                  _formatUptime(uptime),
                  style: MirrorTheme.label(
                    color: Colors.white.withValues(alpha: 0.18),
                    size: 11,
                    weight: FontWeight.w600,
                  ),
                ),
              ],
              if (phase == MirrorPhase.connecting) ...[
                const SizedBox(height: 16),
                const LinearProgressIndicator(
                  color: MirrorTheme.accent,
                  backgroundColor: Colors.transparent,
                ),
              ],
            ],
          ),
        );
      },
    );
  }
}
