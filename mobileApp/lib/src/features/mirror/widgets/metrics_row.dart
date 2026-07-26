import 'package:flutter/material.dart';

import '../../../app/theme.dart';
import '../../../l10n/generated/app_localizations.dart';
import '../../../models/mirror_status.dart';

/// Throughput, latency and frame rate, shown only while streaming.
class MetricsRow extends StatelessWidget {
  const MetricsRow({super.key, required this.metrics});

  final MirrorMetrics metrics;

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);

    return Padding(
      padding: const EdgeInsets.fromLTRB(24, 0, 24, 16),
      child: Row(
        children: [
          _Chip(
            label: l10n.metricThroughput,
            value: '${metrics.throughputMbps.toStringAsFixed(1)} Mbps',
            colour: MirrorTheme.info,
          ),
          const SizedBox(width: 8),
          _Chip(
            label: l10n.metricLatency,
            value: '${metrics.latencyMs} ms',
            colour: MirrorTheme.success,
          ),
          const SizedBox(width: 8),
          _Chip(
            label: l10n.metricFramerate,
            value: metrics.fps.toStringAsFixed(0),
            colour: MirrorTheme.accent,
          ),
        ],
      ),
    );
  }
}

class _Chip extends StatelessWidget {
  const _Chip({required this.label, required this.value, required this.colour});

  final String label;
  final String value;
  final Color colour;

  @override
  Widget build(BuildContext context) {
    return Expanded(
      child: Container(
        padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 10),
        decoration: BoxDecoration(
          color: colour.withValues(alpha: 0.05),
          borderRadius: BorderRadius.circular(12),
          border: Border.all(color: colour.withValues(alpha: 0.08)),
        ),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.start,
          children: [
            Text(
              label.toUpperCase(),
              style: MirrorTheme.label(
                color: colour.withValues(alpha: 0.45),
                size: 7,
                spacing: 1.5,
              ),
            ),
            const SizedBox(height: 4),
            Text(
              value,
              style: MirrorTheme.label(color: colour, size: 14, spacing: 0),
            ),
          ],
        ),
      ),
    );
  }
}
