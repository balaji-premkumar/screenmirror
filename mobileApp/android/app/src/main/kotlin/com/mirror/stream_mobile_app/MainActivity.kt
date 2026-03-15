package com.mirror.stream_mobile_app

import android.app.Activity
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import android.content.IntentFilter
import android.hardware.usb.UsbAccessory
import android.hardware.usb.UsbManager
import android.media.projection.MediaProjectionManager
import android.os.Bundle
import android.os.ParcelFileDescriptor
import io.flutter.embedding.android.FlutterActivity
import io.flutter.plugin.common.MethodChannel

class MainActivity : FlutterActivity() {
    init {
        try {
            System.loadLibrary("c++_shared")
            android.util.Log.i("MirrorUSB", "Preloaded libc++_shared.so successfully")
        } catch (e: UnsatisfiedLinkError) {
            android.util.Log.e("MirrorUSB", "Failed to load c++_shared: ${e.message}")
        }
    }

    private val CHANNEL = "com.mirror.stream/usb"
    private var accessoryFd: ParcelFileDescriptor? = null
    private var methodChannel: MethodChannel? = null
    private var pendingFd: Int? = null
    private var pendingConfig: Map<String, Any>? = null

    private val usbDetachReceiver = object : BroadcastReceiver() {
        override fun onReceive(context: Context?, intent: Intent?) {
            if (intent?.action == UsbManager.ACTION_USB_ACCESSORY_DETACHED) {
                android.util.Log.i("MirrorUSB", "USB accessory detached")
                handleUsbDetachment()
            }
        }
    }

    companion object {
        const val REQUEST_MEDIA_PROJECTION = 1001
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

        flutterEngine?.dartExecutor?.binaryMessenger?.let {
            methodChannel = MethodChannel(it, CHANNEL)
            methodChannel?.setMethodCallHandler { call, result ->
                when (call.method) {
                    "setConfig" -> {
                        pendingConfig = call.arguments as? Map<String, Any>
                        result.success(null)
                    }
                    "stopService" -> {
                        val serviceIntent = Intent(context, com.mirror.stream_mobile_app.service.MirrorForegroundService::class.java)
                        stopService(serviceIntent)
                        result.success(null)
                    }
                    "requestMediaProjection" -> {
                        requestMediaProjection()
                        result.success(null)
                    }
                    "startRustPipeline" -> {
                        // Rust pipeline is started from Dart via FFI, this is a no-op on the native side
                        result.success(null)
                    }
                    "getMetrics" -> {
                        // Metrics are fetched via FFI from Dart directly
                        result.success(null)
                    }
                    "getInitialAccessory" -> {
                        // Let Dart poll for the FD in case it missed the initial intent broadcast during boot
                        result.success(pendingFd)
                        pendingFd = null // Clear it once consumed
                    }
                    else -> result.notImplemented()
                }
            }
        }

        // Register for USB detachment events
        val filter = IntentFilter(UsbManager.ACTION_USB_ACCESSORY_DETACHED)
        if (android.os.Build.VERSION.SDK_INT >= android.os.Build.VERSION_CODES.TIRAMISU) {
            registerReceiver(usbDetachReceiver, filter, RECEIVER_NOT_EXPORTED)
        } else {
            registerReceiver(usbDetachReceiver, filter)
        }

        // Check if launched by USB accessory attachment
        if (intent?.action == UsbManager.ACTION_USB_ACCESSORY_ATTACHED) {
            handleUsbAttachment(intent)
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        if (intent.action == UsbManager.ACTION_USB_ACCESSORY_ATTACHED) {
            handleUsbAttachment(intent)
        }
    }

    private fun handleUsbAttachment(intent: Intent?) {
        val usbManager = getSystemService(USB_SERVICE) as UsbManager

        @Suppress("DEPRECATION")
        val accessory: UsbAccessory? = intent?.getParcelableExtra(UsbManager.EXTRA_ACCESSORY)

        if (accessory != null) {
            accessoryFd = usbManager.openAccessory(accessory)
            val fd = accessoryFd?.fd

            if (fd != null && fd >= 0) {
                android.util.Log.i("MirrorUSB", "Accessory opened: FD=$fd (${accessory.manufacturer} ${accessory.model})")
                pendingFd = fd
                methodChannel?.invokeMethod("onUsbAttached", fd)
            } else {
                android.util.Log.e("MirrorUSB", "Failed to open accessory — FD is invalid")
            }
        } else {
            android.util.Log.w("MirrorUSB", "USB_ACCESSORY_ATTACHED but no accessory in extras")
        }
    }

    private fun handleUsbDetachment() {
        // Notify Flutter that the USB accessory was unplugged
        methodChannel?.invokeMethod("onUsbDetached", null)

        // Stop the mirroring service
        try {
            val serviceIntent = Intent(this, com.mirror.stream_mobile_app.service.MirrorForegroundService::class.java)
            stopService(serviceIntent)
        } catch (e: Exception) {
            android.util.Log.w("MirrorUSB", "Error stopping service on detach: ${e.message}")
        }

        // Close the accessory FD
        try {
            accessoryFd?.close()
            accessoryFd = null
        } catch (e: Exception) {
            android.util.Log.w("MirrorUSB", "Error closing FD on detach: ${e.message}")
        }
        pendingFd = null
    }

    private fun requestMediaProjection() {
        val mpManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        startActivityForResult(mpManager.createScreenCaptureIntent(), REQUEST_MEDIA_PROJECTION)
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode == REQUEST_MEDIA_PROJECTION && resultCode == Activity.RESULT_OK && data != null) {
            // Start the foreground service with the projection result
            val serviceIntent = Intent(this, com.mirror.stream_mobile_app.service.MirrorForegroundService::class.java).apply {
                putExtra(com.mirror.stream_mobile_app.service.MirrorForegroundService.EXTRA_RESULT_CODE, resultCode)
                putExtra(com.mirror.stream_mobile_app.service.MirrorForegroundService.EXTRA_DATA, data)
                
                val res = pendingConfig?.get("resolution") as? String ?: "1080p"
                val bit = pendingConfig?.get("bitrate") as? String ?: "8 Mbps"
                val fps = pendingConfig?.get("fps") as? String ?: "60"
                
                putExtra("resolution", res)
                putExtra("bitrate", bit)
                putExtra("fps", fps)
            }
            startForegroundService(serviceIntent)
            android.util.Log.i("MirrorUSB", "Foreground mirroring service started")
        }
    }

    override fun onDestroy() {
        try {
            unregisterReceiver(usbDetachReceiver)
        } catch (_: Exception) { }
        try {
            accessoryFd?.close()
        } catch (e: Exception) {
            android.util.Log.w("MirrorUSB", "Error closing accessory FD: ${e.message}")
        }
        super.onDestroy()
    }
}
