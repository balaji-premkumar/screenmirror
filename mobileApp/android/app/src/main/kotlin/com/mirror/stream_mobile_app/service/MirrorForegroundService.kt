package com.mirror.stream_mobile_app.service

import android.app.*
// R is generated into the app's namespace, not this subpackage.
import com.mirror.stream_mobile_app.R
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.content.pm.ServiceInfo
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.AudioAttributes
import android.media.AudioFormat
import android.media.AudioPlaybackCaptureConfiguration
import android.media.AudioRecord
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaCodecList
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.IBinder
import android.view.Surface
import androidx.core.app.NotificationCompat
import java.nio.ByteBuffer
import java.nio.ByteOrder
import java.util.concurrent.ExecutorService
import java.util.concurrent.Executors
import java.util.concurrent.TimeUnit

class MirrorForegroundService : Service() {
    private var mediaProjection: MediaProjection? = null
    private var mediaCodec: MediaCodec? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var inputSurface: Surface? = null
    private var drainThread: Thread? = null

    private var audioRecord: AudioRecord? = null
    private var audioThread: Thread? = null

    /** All projection start/stop work runs here, one at a time. */
    private val projectionExecutor: ExecutorService = Executors.newSingleThreadExecutor()

    @Volatile
    private var isRunning = false

    @Volatile
    private var audioRunning = false

    companion object {
        const val CHANNEL_ID = "MirrorForegroundServiceChannel"
        const val EXTRA_RESULT_CODE = "extra_result_code"
        const val EXTRA_DATA = "extra_data"

        const val AUDIO_SAMPLE_RATE = 48000

        init {
            System.loadLibrary("rust_lib_stream_mobile_app")
        }
    }

    // JNI bridge into the Rust USB pipeline. The arrays are reused across
    // frames, so the valid payload is the first `len` bytes.
    private external fun pushToUsb(data: ByteArray, len: Int): Boolean
    private external fun pushAudioToUsb(data: ByteArray, len: Int): Boolean

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "STOP_MIRRORING") {
            android.util.Log.i("MirrorService", "Stop action received from notification")
            stopSelf()
            return START_NOT_STICKY
        }
        return handleStart(intent, flags, startId)
    }

    private fun handleStart(intent: Intent?, flags: Int, startId: Int): Int {
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Activity.RESULT_CANCELED)
            ?: Activity.RESULT_CANCELED
        @Suppress("DEPRECATION")
        val data = intent?.getParcelableExtra<Intent>(EXTRA_DATA)

        val resStr = intent?.getStringExtra("resolution") ?: "1080p"
        val bitStr = intent?.getStringExtra("bitrate") ?: "8 Mbps"
        val fpsStr = intent?.getStringExtra("fps") ?: "60"

        createNotificationChannel()

        val stopIntent = Intent(this, MirrorForegroundService::class.java).apply {
            action = "STOP_MIRRORING"
        }
        val stopPendingIntent = PendingIntent.getService(
            this, 0, stopIntent,
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle(getString(R.string.notification_title))
            .setContentText(getString(R.string.notification_text))
            .setStyle(
                NotificationCompat.BigTextStyle()
                    .bigText(getString(R.string.notification_text_expanded))
            )
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setColor(0xFFFF5722.toInt())
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .addAction(
                android.R.drawable.ic_media_pause,
                getString(R.string.notification_action_stop),
                stopPendingIntent
            )
            .build()

        if (!enterForeground(notification)) {
            stopSelf()
            return START_NOT_STICKY
        }

        if (resultCode == Activity.RESULT_OK && data != null) {
            // Serialised on a single thread: never blocks the caller (setup
            // joins the previous drain thread), and two start intents arriving
            // back to back can no longer both pass an `isRunning` check that
            // is only set at the *end* of startProjection — which used to give
            // two encoders and two VirtualDisplays off one projection.
            //
            // A start while already running is a reconfigure ("Sync
            // Parameters"), not something to drop: startProjection tears the
            // old session down first. Ignoring it left the user having granted
            // capture consent for settings that never took effect.
            projectionExecutor.execute {
                startProjection(resultCode, data, resStr, bitStr, fpsStr)
            }
        }

        return START_NOT_STICKY
    }

    private fun hasAudioPermission(): Boolean =
        Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q &&
            checkSelfPermission(android.Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED

    /**
     * Enter the foreground, declaring the microphone service type only when
     * RECORD_AUDIO is actually held.
     *
     * On Android 14+ declaring FOREGROUND_SERVICE_TYPE_MICROPHONE without the
     * permission throws SecurityException, which killed the whole service —
     * so denying the audio prompt used to cost the user video mirroring too.
     * The permission was only checked later, in startAudioCapture().
     */
    private fun enterForeground(notification: Notification): Boolean {
        try {
            when {
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.UPSIDE_DOWN_CAKE -> {
                    var types = ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION
                    if (hasAudioPermission()) {
                        types = types or ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE
                    } else {
                        android.util.Log.w(
                            "MirrorService",
                            "RECORD_AUDIO not granted — starting without the microphone service type; video only"
                        )
                    }
                    startForeground(1, notification, types)
                }
                Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q ->
                    startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
                else -> startForeground(1, notification)
            }
            return true
        } catch (e: Exception) {
            android.util.Log.e("MirrorService", "startForeground failed: ${e.message}")
        }

        // Fall back to projection only — better a session without sound than
        // no session at all.
        return try {
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
            } else {
                startForeground(1, notification)
            }
            true
        } catch (e2: Exception) {
            android.util.Log.e("MirrorService", "startForeground failed permanently: ${e2.message}")
            false
        }
    }

    private fun startProjection(resultCode: Int, data: Intent, resStr: String, bitStr: String, fpsStr: String) {
        // Stop existing projection if any (for "Sync Parameters" support)
        stopProjection()

        val mpManager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        val projection = mpManager.getMediaProjection(resultCode, data) ?: return
        mediaProjection = projection

        // REQUIRED: Must register a callback on Android 14+ / SDK 34+
        projection.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                android.util.Log.i("MirrorService", "MediaProjection stopped by system")
                stopSelf()
            }
        }, null)

        // Use Configuration to safely detect orientation from a Service context
        val orientation = resources.configuration.orientation
        val isPortrait = orientation == android.content.res.Configuration.ORIENTATION_PORTRAIT

        var targetWidth = when (resStr) {
            "720p" -> 1280
            "2K" -> 2560
            "4K" -> 3840
            else -> 1920
        }
        var targetHeight = when (resStr) {
            "720p" -> 720
            "2K" -> 1440
            "4K" -> 2160
            else -> 1080
        }

        if (isPortrait) {
            val temp = targetWidth
            targetWidth = targetHeight
            targetHeight = temp
        }

        val dpi = resources.displayMetrics.densityDpi
        val bitrate = (bitStr.split(" ").firstOrNull()?.toIntOrNull() ?: 8) * 1024 * 1024
        var fps = fpsStr.split(" ").firstOrNull()?.toIntOrNull() ?: 60

        // Clamp the requested frame rate to what the HEVC encoder actually
        // supports at this resolution — otherwise the codec silently caps or
        // rejects the format and "120 fps" never leaves the phone.
        fps = clampFpsToEncoderCaps(targetWidth, targetHeight, fps)

        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC, targetWidth, targetHeight)
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        format.setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        format.setInteger(MediaFormat.KEY_FRAME_RATE, fps)
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)

        // Real-time operation at the requested frame rate. Without
        // KEY_OPERATING_RATE / max-fps-to-encoder most devices schedule the
        // codec for 30/60 fps regardless of KEY_FRAME_RATE.
        format.setInteger(MediaFormat.KEY_PRIORITY, 0) // 0 = realtime
        format.setInteger(MediaFormat.KEY_LATENCY, 0)  // lowest latency
        format.setInteger(MediaFormat.KEY_OPERATING_RATE, fps)
        format.setFloat("max-fps-to-encoder", fps.toFloat())

        // Re-emit the last frame when the screen is static so the link never
        // goes silent (the desktop drops idle links after 5 s). 100 ms → 10 fps floor.
        format.setLong(MediaFormat.KEY_REPEAT_PREVIOUS_FRAME_AFTER, 100_000L)

        // Prepend VPS/SPS/PPS to every keyframe so the desktop decoder can
        // (re)initialize even if the very first config packet was lost.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) {
            format.setInteger(MediaFormat.KEY_PREPEND_HEADER_TO_SYNC_FRAMES, 1)
        }

        try {
            mediaCodec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_HEVC)
            mediaCodec?.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        } catch (e: Exception) {
            android.util.Log.e("MirrorService", "Encoder configure failed (${e.message}) — retrying without rate hints")
            // Some vendor codecs reject operating-rate hints; retry without them.
            try {
                mediaCodec?.release()
                format.setInteger(MediaFormat.KEY_OPERATING_RATE, 0)
                mediaCodec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_HEVC)
                mediaCodec?.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
            } catch (e2: Exception) {
                android.util.Log.e("MirrorService", "Encoder configure failed permanently: ${e2.message}")
                stopSelf()
                return
            }
        }
        inputSurface = mediaCodec?.createInputSurface()
        mediaCodec?.start()

        virtualDisplay = projection.createVirtualDisplay(
            "MirrorDisplay", targetWidth, targetHeight, dpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            inputSurface, null, null
        )

        isRunning = true
        android.util.Log.i(
            "MirrorService",
            "Encoder started: ${targetWidth}x${targetHeight} @ ${fps}fps ${bitrate / 1024 / 1024}Mbps HEVC (Portrait: $isPortrait)"
        )
        drainThread = Thread { drainEncoder() }.also { it.start() }

        startAudioCapture(projection)
    }

    private fun clampFpsToEncoderCaps(width: Int, height: Int, requestedFps: Int): Int {
        try {
            val codecList = MediaCodecList(MediaCodecList.REGULAR_CODECS)
            for (info in codecList.codecInfos) {
                if (!info.isEncoder) continue
                val type = info.supportedTypes.firstOrNull {
                    it.equals(MediaFormat.MIMETYPE_VIDEO_HEVC, ignoreCase = true)
                } ?: continue
                val caps = info.getCapabilitiesForType(type).videoCapabilities ?: continue
                if (!caps.isSizeSupported(width, height)) continue
                val range = caps.getSupportedFrameRatesFor(width, height)
                val maxFps = range.upper.toInt()
                if (requestedFps > maxFps) {
                    android.util.Log.w(
                        "MirrorService",
                        "Encoder ${info.name} caps ${width}x${height} at ${maxFps}fps — clamping from ${requestedFps}fps"
                    )
                    return maxFps
                }
                return requestedFps
            }
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Capability probe failed: ${e.message}")
        }
        return requestedFps
    }

    private fun drainEncoder() {
        val bufferInfo = MediaCodec.BufferInfo()
        var frameCount = 0L
        // Grown as needed and reused. Allocating a ByteArray per frame put
        // ~4 MB/s of garbage through the collector at 120 fps, on the one
        // thread that must not pause.
        var outData = ByteArray(256 * 1024)
        while (isRunning) {
            val encoder = mediaCodec ?: break
            val outputBufferIndex = try {
                encoder.dequeueOutputBuffer(bufferInfo, 10_000)
            } catch (e: Exception) {
                break
            }

            if (outputBufferIndex >= 0) {
                try {
                    val outputBuffer = encoder.getOutputBuffer(outputBufferIndex)
                    if (outputBuffer != null && bufferInfo.size > 0) {
                        outputBuffer.position(bufferInfo.offset)
                        outputBuffer.limit(bufferInfo.offset + bufferInfo.size)

                        if (outData.size < bufferInfo.size) {
                            outData = ByteArray(bufferInfo.size)
                        }
                        outputBuffer.get(outData, 0, bufferInfo.size)

                        // Push encoded H.265 NAL units to Rust for USB transmission
                        val sent = pushToUsb(outData, bufferInfo.size)
                        if (!sent && frameCount % 120 == 0L) {
                            android.util.Log.w("MirrorService", "Frame $frameCount: USB pipeline not ready, frame dropped")
                        }

                        frameCount++
                        if (frameCount % 600 == 0L) {
                            android.util.Log.d("MirrorService", "Encoded $frameCount frames (${bufferInfo.size} bytes last)")
                        }
                    }
                } finally {
                    // Always release — leaking indices for empty/EOS buffers
                    // exhausts the codec's buffer pool and freezes the stream.
                    try {
                        encoder.releaseOutputBuffer(outputBufferIndex, false)
                    } catch (_: Exception) {
                    }
                }
            }
        }
        android.util.Log.i("MirrorService", "Encoder drain loop exited after $frameCount frames")
    }

    /**
     * Capture device audio (media/game output) via AudioPlaybackCapture and
     * push it to the USB pipeline as f32 LE mono 48 kHz — the wire format the
     * desktop plays back and forwards to OBS.
     */
    private fun startAudioCapture(projection: MediaProjection) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.Q) {
            android.util.Log.w("MirrorService", "AudioPlaybackCapture requires Android 10+ — audio disabled")
            return
        }
        // Checked inline rather than through hasAudioPermission(): lint only
        // recognises a direct checkSelfPermission call here, and keeping it
        // visible is worth more than deduplicating a one-liner. This must stay
        // consistent with the service-type decision in enterForeground().
        if (checkSelfPermission(android.Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED
        ) {
            android.util.Log.w("MirrorService", "RECORD_AUDIO not granted — audio disabled")
            return
        }

        try {
            val captureConfig = AudioPlaybackCaptureConfiguration.Builder(projection)
                .addMatchingUsage(AudioAttributes.USAGE_MEDIA)
                .addMatchingUsage(AudioAttributes.USAGE_GAME)
                .addMatchingUsage(AudioAttributes.USAGE_UNKNOWN)
                .build()

            val audioFormat = AudioFormat.Builder()
                .setEncoding(AudioFormat.ENCODING_PCM_FLOAT)
                .setSampleRate(AUDIO_SAMPLE_RATE)
                .setChannelMask(AudioFormat.CHANNEL_IN_MONO)
                .build()

            val minBuf = AudioRecord.getMinBufferSize(
                AUDIO_SAMPLE_RATE,
                AudioFormat.CHANNEL_IN_MONO,
                AudioFormat.ENCODING_PCM_FLOAT
            ).coerceAtLeast(4096)

            val record = AudioRecord.Builder()
                .setAudioFormat(audioFormat)
                .setBufferSizeInBytes(minBuf * 2)
                .setAudioPlaybackCaptureConfig(captureConfig)
                .build()

            record.startRecording()
            audioRecord = record
            audioRunning = true

            audioThread = Thread {
                val floats = FloatArray(1024)
                // Both buffers are allocated once and reused; only the first
                // `read * 4` bytes are handed across the JNI boundary.
                val out = ByteArray(floats.size * 4)
                val bytes = ByteBuffer.wrap(out).order(ByteOrder.LITTLE_ENDIAN)
                while (audioRunning) {
                    val read = record.read(floats, 0, floats.size, AudioRecord.READ_BLOCKING)
                    if (read > 0) {
                        bytes.clear()
                        for (i in 0 until read) bytes.putFloat(floats[i])
                        pushAudioToUsb(out, read * 4)
                    } else if (read < 0) {
                        android.util.Log.w("MirrorService", "AudioRecord read error: $read")
                        break
                    }
                }
            }.also { it.start() }

            android.util.Log.i("MirrorService", "System audio capture started (48kHz f32 mono)")
        } catch (e: Exception) {
            android.util.Log.e("MirrorService", "Audio capture failed: ${e.message}")
        }
    }

    private fun stopAudioCapture() {
        audioRunning = false
        try {
            audioThread?.join(500)
        } catch (_: Exception) {
        }
        audioThread = null
        try {
            audioRecord?.stop()
        } catch (_: Exception) {
        }
        try {
            audioRecord?.release()
        } catch (_: Exception) {
        }
        audioRecord = null
    }

    private fun stopProjection() {
        isRunning = false
        stopAudioCapture()

        // Wait for the drain loop to observe isRunning = false — bounded, and
        // never called from the main thread (see handleStart).
        try {
            drainThread?.join(500)
        } catch (_: Exception) {
        }
        drainThread = null

        try {
            virtualDisplay?.release()
            virtualDisplay = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Error releasing virtual display: ${e.message}")
        }

        try {
            mediaCodec?.stop()
        } catch (_: Exception) {
        }

        try {
            mediaCodec?.release()
            mediaCodec = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Error releasing codec: ${e.message}")
        }

        try {
            inputSurface?.release()
            inputSurface = null
        } catch (_: Exception) {
        }

        try {
            mediaProjection?.stop()
            mediaProjection = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Error stopping projection: ${e.message}")
        }
    }

    override fun onDestroy() {
        android.util.Log.i("MirrorService", "Destroying service...")
        // Let any in-flight projection setup finish before tearing it down,
        // otherwise stopProjection() can race a half-built session and leak
        // the encoder or the VirtualDisplay.
        projectionExecutor.shutdown()
        try {
            if (!projectionExecutor.awaitTermination(2, TimeUnit.SECONDS)) {
                projectionExecutor.shutdownNow()
            }
        } catch (_: InterruptedException) {
            projectionExecutor.shutdownNow()
        }
        stopProjection()
        android.util.Log.i("MirrorService", "Service destroyed, resources released")
        super.onDestroy()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val serviceChannel = NotificationChannel(
                CHANNEL_ID,
                getString(R.string.notification_channel_name),
                NotificationManager.IMPORTANCE_HIGH
            ).apply {
                description = getString(R.string.notification_channel_description)
                lockscreenVisibility = NotificationCompat.VISIBILITY_PUBLIC
            }
            getSystemService(NotificationManager::class.java).createNotificationChannel(serviceChannel)
        }
    }
}
