import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:stream_mobile_app/src/l10n/generated/app_localizations.dart';
import 'package:stream_mobile_app/src/models/mirror_status.dart';

void main() {
  test('metrics parse from the native JSON shape', () {
    final metrics = MirrorMetrics.fromJson(const {
      'throughput_mbps': 12.5,
      'encoding_latency_ms': 8,
      'fps_actual': 59.7,
    });

    expect(metrics.throughputMbps, 12.5);
    expect(metrics.latencyMs, 8);
    expect(metrics.fps, closeTo(59.7, 0.001));
  });

  test('missing fields read as zero rather than throwing', () {
    // The native side omits fields before the first frame arrives. Throwing
    // here would kill the polling timer that produces every later reading.
    const metrics = MirrorMetrics.zero;
    expect(MirrorMetrics.fromJson(const {}).throughputMbps, metrics.throughputMbps);
    expect(MirrorMetrics.fromJson(const {}).latencyMs, 0);
    expect(MirrorMetrics.fromJson(const {}).fps, 0);
  });

  test('integers where doubles are expected still parse', () {
    // serde_json emits `0` rather than `0.0` for a whole number, and a plain
    // `as double` cast on that throws.
    final metrics = MirrorMetrics.fromJson(const {
      'throughput_mbps': 8,
      'fps_actual': 60,
    });
    expect(metrics.throughputMbps, 8.0);
    expect(metrics.fps, 60.0);
  });

  test('every phase has a description', () async {
    final l10n = await AppLocalizations.delegate.load(const Locale('en'));
    for (final phase in MirrorPhase.values) {
      expect(describePhase(l10n, phase), isNotEmpty, reason: '$phase');
    }
  });
}
