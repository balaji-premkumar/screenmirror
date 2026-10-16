import 'package:flutter/material.dart';

import '../l10n/generated/app_localizations.dart';

/// Where the app is in the connect-then-capture sequence.
///
/// Two separate things have to happen before anything streams: the USB cable
/// has to be connected ([linked]), and the user has to approve screen capture
/// on the phone ([streaming]). Collapsing them into one boolean is why an
/// earlier version claimed "Streaming to PC" while the consent dialog was
/// still on screen and might yet be declined.
enum MirrorPhase {
  /// No cable, or the cable was pulled.
  idle,

  /// A cable is present and the connection is being opened.
  connecting,

  /// The connection is open. Nothing is being captured yet.
  linked,

  /// Capture was approved and video and audio are on the wire.
  streaming,

  /// Something failed that the user has to resolve.
  error,
}

/// Live throughput figures from the native side.
@immutable
class MirrorMetrics {
  const MirrorMetrics({
    this.throughputMbps = 0,
    this.latencyMs = 0,
    this.fps = 0,
  });

  final double throughputMbps;
  final int latencyMs;
  final double fps;

  static const zero = MirrorMetrics();

  factory MirrorMetrics.fromJson(Map<String, dynamic> json) => MirrorMetrics(
        throughputMbps: (json['throughput_mbps'] as num? ?? 0).toDouble(),
        latencyMs: (json['encoding_latency_ms'] as num? ?? 0).toInt(),
        fps: (json['fps_actual'] as num? ?? 0).toDouble(),
      );
}

/// Which message to show for the current phase.
///
/// Some phases have more than one sensible message — [MirrorPhase.linked]
/// means something different before capture has been attempted than after it
/// was declined — so the controller supplies an override where it has one.
String describePhase(AppLocalizations l10n, MirrorPhase phase) => switch (phase) {
      MirrorPhase.idle => l10n.statusWaitingForUsb,
      MirrorPhase.connecting => l10n.statusOpeningPipeline,
      MirrorPhase.linked => l10n.statusReadyPressStart,
      MirrorPhase.streaming => l10n.statusStreaming,
      MirrorPhase.error => l10n.statusPipelineFailed,
    };
