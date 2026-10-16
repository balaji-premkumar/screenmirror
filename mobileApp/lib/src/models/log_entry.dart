import 'package:flutter/material.dart';

import '../l10n/generated/app_localizations.dart';

/// What kind of event a log line describes. Drives its colour, nothing else.
enum LogSeverity { info, success, warning, error }

/// A stable identifier for something that happened.
///
/// The wording lives in the ARB files, not here. This is the same split the
/// desktop backend uses: emitting a code rather than a sentence is what lets
/// the display layer choose the language, and it means a message can be
/// reworded without touching the code that emits it.
enum LogCode {
  appStarted(LogSeverity.info),
  audioPermissionGranted(LogSeverity.success),
  audioPermissionDenied(LogSeverity.warning),
  notificationPermissionDenied(LogSeverity.warning),
  usbListening(LogSeverity.info),
  usbAttached(LogSeverity.info),
  usbRecovered(LogSeverity.info),
  usbDetached(LogSeverity.warning),
  usbQueryFailed(LogSeverity.warning),
  nativeLoading(LogSeverity.info),
  nativeReady(LogSeverity.success),
  nativeFailed(LogSeverity.error),
  pipelineStarting(LogSeverity.info),
  pipelineStarted(LogSeverity.success),
  pipelineFailed(LogSeverity.error),
  pipelineStopped(LogSeverity.info),
  configReceived(LogSeverity.info),
  captureRequested(LogSeverity.info),
  captureRequestFailed(LogSeverity.error),
  captureApproved(LogSeverity.success),
  captureDenied(LogSeverity.warning),
  streaming(LogSeverity.success),
  stopRequested(LogSeverity.info),
  stopped(LogSeverity.info),
  connectionLost(LogSeverity.error);

  const LogCode(this.severity);

  final LogSeverity severity;
}

/// One line in the activity panel.
@immutable
class LogEntry {
  const LogEntry({required this.time, required this.code, this.detail});

  final DateTime time;
  final LogCode code;

  /// Fills the `{error}` or `{state}` placeholder for codes that take one.
  final String? detail;

  LogSeverity get severity => code.severity;

  /// `HH:MM:SS`, which is all the precision a human reading a list needs.
  String get timestamp =>
      '${time.hour.toString().padLeft(2, '0')}:'
      '${time.minute.toString().padLeft(2, '0')}:'
      '${time.second.toString().padLeft(2, '0')}';
}

/// Resolves a log entry into text in the user's language.
///
/// The switch is exhaustive over [LogCode], so adding a code without adding
/// its wording is a compile error rather than a blank line in the UI. That is
/// the whole reason the codes are an enum and not strings.
String describeLogEntry(AppLocalizations l10n, LogEntry entry) {
  final detail = entry.detail ?? '';
  return switch (entry.code) {
    LogCode.appStarted => l10n.eventAppStarted,
    LogCode.audioPermissionGranted => l10n.eventAudioPermissionGranted,
    LogCode.audioPermissionDenied => l10n.eventAudioPermissionDenied,
    LogCode.notificationPermissionDenied => l10n.eventNotificationPermissionDenied,
    LogCode.usbListening => l10n.eventUsbListening,
    LogCode.usbAttached => l10n.eventUsbAttached,
    LogCode.usbRecovered => l10n.eventUsbRecovered,
    LogCode.usbDetached => l10n.eventUsbDetached,
    LogCode.usbQueryFailed => l10n.eventUsbQueryFailed(detail),
    LogCode.nativeLoading => l10n.eventNativeLoading,
    LogCode.nativeReady => l10n.eventNativeReady,
    LogCode.nativeFailed => l10n.eventNativeFailed(detail),
    LogCode.pipelineStarting => l10n.eventPipelineStarting,
    LogCode.pipelineStarted => l10n.eventPipelineStarted,
    LogCode.pipelineFailed => l10n.eventPipelineFailed(detail),
    LogCode.pipelineStopped => l10n.eventPipelineStopped,
    LogCode.configReceived => l10n.eventConfigReceived,
    LogCode.captureRequested => l10n.eventCaptureRequested,
    LogCode.captureRequestFailed => l10n.eventCaptureRequestFailed(detail),
    LogCode.captureApproved => l10n.eventCaptureApproved,
    LogCode.captureDenied => l10n.eventCaptureDenied,
    LogCode.streaming => l10n.eventStreaming,
    LogCode.stopRequested => l10n.eventStopRequested,
    LogCode.stopped => l10n.eventStopped,
    LogCode.connectionLost => l10n.eventConnectionLost(detail),
  };
}
