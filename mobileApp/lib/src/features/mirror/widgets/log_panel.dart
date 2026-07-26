import 'package:flutter/material.dart';

import '../../../app/theme.dart';
import '../../../l10n/generated/app_localizations.dart';
import '../../../models/log_entry.dart';

/// Scrolling list of what the app has done, pinned to the newest entry.
class LogPanel extends StatefulWidget {
  const LogPanel({super.key, required this.entries});

  final List<LogEntry> entries;

  @override
  State<LogPanel> createState() => _LogPanelState();
}

class _LogPanelState extends State<LogPanel> {
  final _scroll = ScrollController();

  @override
  void didUpdateWidget(LogPanel oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (widget.entries.length != oldWidget.entries.length) {
      _scrollToEnd();
    }
  }

  void _scrollToEnd() {
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_scroll.hasClients) return;
      _scroll.animateTo(
        _scroll.position.maxScrollExtent,
        duration: const Duration(milliseconds: 100),
        curve: Curves.easeOut,
      );
    });
  }

  @override
  void dispose() {
    _scroll.dispose();
    super.dispose();
  }

  static Color _severityColour(LogSeverity severity) => switch (severity) {
        LogSeverity.error => MirrorTheme.danger,
        LogSeverity.warning => MirrorTheme.warning,
        LogSeverity.success => MirrorTheme.success,
        LogSeverity.info => Colors.grey,
      };

  @override
  Widget build(BuildContext context) {
    final l10n = AppLocalizations.of(context);

    return Container(
      margin: const EdgeInsets.fromLTRB(16, 12, 16, 0),
      decoration: BoxDecoration(
        color: MirrorTheme.panel,
        borderRadius: const BorderRadius.vertical(top: Radius.circular(16)),
        border: Border.all(color: Colors.white.withValues(alpha: 0.03)),
      ),
      child: Column(
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(16, 12, 16, 8),
            child: Row(
              mainAxisAlignment: MainAxisAlignment.spaceBetween,
              children: [
                Text(
                  l10n.logTitle.toUpperCase(),
                  style: MirrorTheme.label(
                    color: Colors.white.withValues(alpha: 0.2),
                  ),
                ),
                Text(
                  '${widget.entries.length}',
                  style: TextStyle(
                    color: Colors.white.withValues(alpha: 0.12),
                    fontSize: 9,
                  ),
                ),
              ],
            ),
          ),
          Divider(height: 1, color: Colors.white.withValues(alpha: 0.03)),
          Expanded(
            child: widget.entries.isEmpty
                ? Center(
                    child: Text(
                      l10n.logEmpty,
                      style: TextStyle(
                        color: Colors.white.withValues(alpha: 0.1),
                        fontSize: 11,
                      ),
                    ),
                  )
                : ListView.builder(
                    controller: _scroll,
                    padding: const EdgeInsets.symmetric(horizontal: 12, vertical: 8),
                    itemCount: widget.entries.length,
                    itemBuilder: (context, index) {
                      final entry = widget.entries[index];
                      final colour = _severityColour(entry.severity);
                      return Padding(
                        padding: const EdgeInsets.symmetric(vertical: 2),
                        child: Row(
                          crossAxisAlignment: CrossAxisAlignment.start,
                          children: [
                            SizedBox(
                              width: 50,
                              child: Text(
                                entry.timestamp,
                                style: MirrorTheme.monospace.copyWith(
                                  color: Colors.white.withValues(alpha: 0.12),
                                  fontSize: 9,
                                ),
                              ),
                            ),
                            Container(
                              width: 4,
                              height: 4,
                              margin: const EdgeInsets.only(top: 4, right: 8),
                              decoration: BoxDecoration(
                                shape: BoxShape.circle,
                                color: colour.withValues(alpha: 0.7),
                              ),
                            ),
                            Expanded(
                              child: Text(
                                describeLogEntry(l10n, entry),
                                style: MirrorTheme.monospace.copyWith(
                                  color: Colors.white.withValues(alpha: 0.5),
                                  fontSize: 10,
                                  height: 1.4,
                                ),
                              ),
                            ),
                          ],
                        ),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }
}
