import 'package:flutter/material.dart';

import '../../app/theme.dart';
import '../../l10n/generated/app_localizations.dart';
import '../../models/mirror_status.dart';
import 'mirror_controller.dart';
import 'widgets/log_panel.dart';
import 'widgets/metrics_row.dart';
import 'widgets/mirror_footer.dart';
import 'widgets/mirror_header.dart';
import 'widgets/status_card.dart';

/// The app's only screen.
///
/// Owns the animation tickers and the controller's lifetime, and nothing else
/// — every piece of state it displays comes from [MirrorController], and every
/// piece of the layout is its own widget.
class MirrorScreen extends StatefulWidget {
  const MirrorScreen({super.key});

  @override
  State<MirrorScreen> createState() => _MirrorScreenState();
}

class _MirrorScreenState extends State<MirrorScreen> with TickerProviderStateMixin {
  late final MirrorController _controller;
  late final AnimationController _pulse;
  late final AnimationController _glow;

  @override
  void initState() {
    super.initState();

    _pulse = AnimationController(
      vsync: this,
      duration: const Duration(milliseconds: 1200),
    )..repeat(reverse: true);

    _glow = AnimationController(
      vsync: this,
      duration: const Duration(seconds: 3),
    )..repeat(reverse: true);

    _controller = MirrorController();
    _controller.start();
  }

  @override
  void dispose() {
    _controller.dispose();
    _pulse.dispose();
    _glow.dispose();
    super.dispose();
  }

  /// Picks the status line, preferring an override the controller set for a
  /// situation the phase alone cannot describe.
  String _statusMessage(AppLocalizations l10n) => switch (_controller.statusOverrideKey) {
        'awaitingApproval' => l10n.statusWaitingForApproval,
        'captureDeclined' => l10n.statusCaptureDeclined,
        'ready' => l10n.statusReadyForCapture,
        _ when _controller.phase == MirrorPhase.error && !_controller.nativeReady =>
          l10n.statusNativeMissing,
        _ => describePhase(l10n, _controller.phase),
      };

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);

    return Scaffold(
      body: SafeArea(
        child: ListenableBuilder(
          listenable: _controller,
          builder: (context, _) {
            return Column(
              children: [
                MirrorHeader(
                  isStreaming: _controller.isStreaming,
                  nativeReady: _controller.nativeReady,
                  pulse: _pulse,
                ),
                StatusCard(
                  phase: _controller.phase,
                  message: _statusMessage(l10n),
                  uptime: _controller.uptime,
                  glow: _glow,
                ),
                if (_controller.isStreaming) MetricsRow(metrics: _controller.metrics),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: 24),
                  child: Divider(color: MirrorTheme.divider(context), height: 1),
                ),
                Expanded(child: LogPanel(entries: _controller.logs)),
                MirrorFooter(isStreaming: _controller.isStreaming),
              ],
            );
          },
        ),
      ),
    );
  }
}
