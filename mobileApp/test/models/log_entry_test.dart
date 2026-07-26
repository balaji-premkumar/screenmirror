import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:stream_mobile_app/src/l10n/generated/app_localizations.dart';
import 'package:stream_mobile_app/src/models/log_entry.dart';

void main() {
  late AppLocalizations l10n;

  setUpAll(() async {
    l10n = await AppLocalizations.delegate.load(const Locale('en'));
  });

  test('every log code resolves to real text', () {
    // `describeLogEntry` switches exhaustively over LogCode, so a missing case
    // is already a compile error. What this catches is the other half: an ARB
    // key that exists but was left empty, which the compiler cannot see.
    for (final code in LogCode.values) {
      final entry = LogEntry(time: DateTime(2026), code: code, detail: 'detail');
      final text = describeLogEntry(l10n, entry);
      expect(text, isNotEmpty, reason: '$code has no wording');
      expect(
        text,
        isNot(contains('{')),
        reason: '$code left a placeholder unsubstituted',
      );
    }
  });

  test('codes that take a detail actually include it', () {
    const withDetail = [
      LogCode.usbQueryFailed,
      LogCode.nativeFailed,
      LogCode.pipelineFailed,
      LogCode.captureRequestFailed,
      LogCode.connectionLost,
    ];

    for (final code in withDetail) {
      final entry = LogEntry(
        time: DateTime(2026),
        code: code,
        detail: 'SOMETHING_SPECIFIC',
      );
      expect(
        describeLogEntry(l10n, entry),
        contains('SOMETHING_SPECIFIC'),
        reason: '$code drops the detail it was given',
      );
    }
  });

  test('a missing detail does not crash or leave a placeholder', () {
    final entry = LogEntry(time: DateTime(2026), code: LogCode.nativeFailed);
    final text = describeLogEntry(l10n, entry);
    expect(text, isNotEmpty);
    expect(text, isNot(contains('{error}')));
  });

  test('timestamps are zero-padded to a fixed width', () {
    // The log lines are laid out in a fixed-width column; an unpadded hour
    // would make every row before 10:00 shift left.
    final entry = LogEntry(
      time: DateTime(2026, 1, 1, 9, 5, 3),
      code: LogCode.appStarted,
    );
    expect(entry.timestamp, '09:05:03');
    expect(entry.timestamp.length, 8);
  });

  test('severity drives the colour, so every code declares one', () {
    for (final code in LogCode.values) {
      expect(code.severity, isNotNull);
    }
    expect(LogCode.nativeFailed.severity, LogSeverity.error);
    expect(LogCode.captureApproved.severity, LogSeverity.success);
    expect(LogCode.audioPermissionDenied.severity, LogSeverity.warning);
  });
}
