package com.mirror.stream_mobile_app.service

import android.app.*
import android.content.Context
import android.content.Intent
import android.hardware.display.DisplayManager
import android.hardware.display.VirtualDisplay
import android.media.MediaCodec
import android.media.MediaCodecInfo
import android.media.MediaFormat
import android.media.projection.MediaProjection
import android.media.projection.MediaProjectionManager
import android.os.Build
import android.os.IBinder
import android.view.Surface
import android.content.pm.ServiceInfo
import androidx.core.app.NotificationCompat
import java.nio.ByteBuffer

class MirrorForegroundService : Service() {
    private var mediaProjection: MediaProjection? = null
    private var mediaCodec: MediaCodec? = null
    private var virtualDisplay: VirtualDisplay? = null
    private var inputSurface: Surface? = null
    private var isRunning = false

    companion object {
        const val CHANNEL_ID = "MirrorForegroundServiceChannel"
        const val EXTRA_RESULT_CODE = "extra_result_code"
        const val EXTRA_DATA = "extra_data"
        
        init {
            System.loadLibrary("rust_lib_stream_mobile_app")
        }
    }

    // JNI bridge to push encoded H.265 video data into the Rust USB write pipeline
    private external fun pushToUsb(data: ByteArray): Boolean

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == "STOP_MIRRORING") {
            android.util.Log.i("MirrorService", "Stop action received from notification")
            stopSelf()
            return START_NOT_STICKY
        }
        // ... rest handled by primary block below
        return handleStart(intent, flags, startId)
    }

    private fun handleStart(intent: Intent?, flags: Int, startId: Int): Int {
        val resultCode = intent?.getIntExtra(EXTRA_RESULT_CODE, Activity.RESULT_CANCELED) ?: Activity.RESULT_CANCELED
        @Suppress("DEPRECATION")
        val data = intent?.getParcelableExtra<Intent>(EXTRA_DATA)

        val resStr = intent?.getStringExtra("resolution") ?: "1080p"
        val bitStr = intent?.getStringExtra("bitrate") ?: "8 Mbps"
        val fpsStr = intent?.getStringExtra("fps") ?: "60"

        createNotificationChannel()

        val stopIntent = Intent(this, MirrorForegroundService::class.java).apply {
            action = "STOP_MIRRORING"
        }
        val stopPendingIntent = android.app.PendingIntent.getService(
            this, 0, stopIntent,
            android.app.PendingIntent.FLAG_UPDATE_CURRENT or android.app.PendingIntent.FLAG_IMMUTABLE
        )

        val notification = NotificationCompat.Builder(this, CHANNEL_ID)
            .setContentTitle("⚠️ Screen Mirroring Active")
            .setContentText("Your screen is being streamed to PC via USB")
            .setStyle(NotificationCompat.BigTextStyle()
                .bigText("Your screen is being streamed to PC via USB. Tap 'Stop' to end the session."))
            .setSmallIcon(android.R.drawable.ic_dialog_alert)
            .setColor(0xFFFF5722.toInt())
            .setOngoing(true)
            .setPriority(NotificationCompat.PRIORITY_HIGH)
            .setCategory(NotificationCompat.CATEGORY_SERVICE)
            .setVisibility(NotificationCompat.VISIBILITY_PUBLIC)
            .addAction(android.R.drawable.ic_media_pause, "Stop Mirroring", stopPendingIntent)
            .build()

        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
            startForeground(1, notification, ServiceInfo.FOREGROUND_SERVICE_TYPE_MEDIA_PROJECTION)
        } else {
            startForeground(1, notification)
        }

        if (resultCode == Activity.RESULT_OK && data != null) {
            if (!isRunning) {
                startProjection(resultCode, data, resStr, bitStr, fpsStr)
            } else {
                android.util.Log.i("MirrorService", "Projection already running, ignoring start request")
            }
        }

        return START_NOT_STICKY
    }

    private fun startProjection(resultCode: Int, data: Intent, resStr: String, bitStr: String, fpsStr: String) {
        // Stop existing projection if any (for "Sync Parameters" support)
        stopProjection()

        val mpManager = getSystemService(Context.MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        mediaProjection = mpManager.getMediaProjection(resultCode, data) ?: return

        // REQUIRED: Must register a callback on Android 14+ / SDK 34+
        mediaProjection?.registerCallback(object : MediaProjection.Callback() {
            override fun onStop() {
                android.util.Log.i("MirrorService", "MediaProjection stopped by system")
                stopSelf()
            }
        }, null)

        val width = if (resStr == "720p") 1280 else if (resStr == "2K") 2560 else if (resStr == "4K") 3840 else 1920
        val height = if (resStr == "720p") 720 else if (resStr == "2K") 1440 else if (resStr == "4K") 2160 else 1080
        val dpi = 400
        val bitrate = (bitStr.split(" ").firstOrNull()?.toIntOrNull() ?: 8) * 1024 * 1024
        val fps = fpsStr.split(" ").firstOrNull()?.toIntOrNull() ?: 60

        val format = MediaFormat.createVideoFormat(MediaFormat.MIMETYPE_VIDEO_HEVC, width, height)
        format.setInteger(MediaFormat.KEY_COLOR_FORMAT, MediaCodecInfo.CodecCapabilities.COLOR_FormatSurface)
        format.setInteger(MediaFormat.KEY_BIT_RATE, bitrate)
        format.setInteger(MediaFormat.KEY_FRAME_RATE, fps)
        format.setInteger(MediaFormat.KEY_I_FRAME_INTERVAL, 1)

        // Request hardware encoder for low latency
        format.setInteger(MediaFormat.KEY_PRIORITY, 0) // 0 = realtime
        format.setInteger(MediaFormat.KEY_LATENCY, 0)  // lowest latency

        mediaCodec = MediaCodec.createEncoderByType(MediaFormat.MIMETYPE_VIDEO_HEVC)
        mediaCodec?.configure(format, null, null, MediaCodec.CONFIGURE_FLAG_ENCODE)
        inputSurface = mediaCodec?.createInputSurface()
        mediaCodec?.start()

        virtualDisplay = mediaProjection?.createVirtualDisplay(
            "MirrorDisplay", width, height, dpi,
            DisplayManager.VIRTUAL_DISPLAY_FLAG_AUTO_MIRROR,
            inputSurface, null, null
        )

        isRunning = true
        android.util.Log.i("MirrorService", "Encoder started: ${width}x${height} @ ${bitrate/1024/1024}Mbps HEVC")
        Thread { drainEncoder() }.start()
    }

    private fun drainEncoder() {
        val bufferInfo = MediaCodec.BufferInfo()
        var frameCount = 0L
        while (isRunning) {
            val encoder = mediaCodec ?: break
            val outputBufferIndex = try { 
                encoder.dequeueOutputBuffer(bufferInfo, 10000) 
            } catch (e: Exception) { -1 }

            if (outputBufferIndex >= 0) {
                val outputBuffer = encoder.getOutputBuffer(outputBufferIndex)
                if (outputBuffer != null && bufferInfo.size > 0) {
                    outputBuffer.position(bufferInfo.offset)
                    outputBuffer.limit(bufferInfo.offset + bufferInfo.size)
                    
                    val outData = ByteArray(bufferInfo.size)
                    outputBuffer.get(outData)
                    
                    // Push encoded H.265 NAL units to Rust for USB transmission
                    val sent = pushToUsb(outData)
                    if (!sent) {
                        android.util.Log.w("MirrorService", "Frame $frameCount: USB buffer full, frame dropped")
                    }
                    
                    frameCount++
                    if (frameCount % 300 == 0L) {
                        android.util.Log.d("MirrorService", "Encoded $frameCount frames (${bufferInfo.size} bytes last)")
                    }
                    
                    encoder.releaseOutputBuffer(outputBufferIndex, false)
                }
            }
        }
        android.util.Log.i("MirrorService", "Encoder drain loop exited after $frameCount frames")
    }

    private fun stopProjection() {
        isRunning = false
        // Give a small window for the drain loop to see isRunning = false
        try { Thread.sleep(50) } catch (_: Exception) {}

        try {
            virtualDisplay?.release()
            virtualDisplay = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Error releasing virtual display: ${e.message}")
        }

        try {
            mediaCodec?.stop()
        } catch (e: Exception) {}

        try {
            mediaCodec?.release()
            mediaCodec = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorService", "Error releasing codec: ${e.message}")
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
        stopProjection()
        android.util.Log.i("MirrorService", "Service destroyed, resources released")
        super.onDestroy()
    }

    private fun createNotificationChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val serviceChannel = NotificationChannel(
                CHANNEL_ID, "Screen Mirroring", NotificationManager.IMPORTANCE_HIGH
            ).apply {
                description = "Shows when your screen is being mirrored to a PC"
                lockscreenVisibility = NotificationCompat.VISIBILITY_PUBLIC
            }
            getSystemService(NotificationManager::class.java).createNotificationChannel(serviceChannel)
        }
    }
}
