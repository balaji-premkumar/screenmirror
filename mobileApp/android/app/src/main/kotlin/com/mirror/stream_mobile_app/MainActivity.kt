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
import androidx.core.content.ContextCompat
import io.flutter.embedding.android.FlutterActivity
import io.flutter.embedding.engine.FlutterEngine
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

    // configureFlutterEngine is the guaranteed-safe hook for channel setup —
    // in onCreate the engine may not be attached yet, which used to leave
    // every channel call silently dead on cold start.
    override fun configureFlutterEngine(flutterEngine: FlutterEngine) {
        super.configureFlutterEngine(flutterEngine)
        methodChannel = MethodChannel(flutterEngine.dartExecutor.binaryMessenger, CHANNEL)
        methodChannel?.setMethodCallHandler { call, result ->
            when (call.method) {
                "setConfig" -> {
                    @Suppress("UNCHECKED_CAST")
                    pendingConfig = call.arguments as? Map<String, Any>
                    result.success(null)
                }
                "stopService" -> {
                    val serviceIntent = Intent(this, com.mirror.stream_mobile_app.service.MirrorForegroundService::class.java)
                    stopService(serviceIntent)
                    result.success(null)
                }
                "requestMediaProjection" -> {
                    requestMediaProjection()
                    result.success(null)
                }
                "getInitialAccessory" -> {
                    // Let Dart poll for the FD in case it missed the initial intent broadcast
                    result.success(pendingFd)
                    pendingFd = null // Clear it once consumed
                }
                else -> result.notImplemented()
            }
        }
    }

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)

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

    /**
     * Close an accessory fd that Dart never took ownership of.
     *
     * `pendingFd` is only non-null while ownership still sits on this side:
     * it is cleared the moment Dart acknowledges `onUsbAttached` or drains
     * `getInitialAccessory`. Discarding it without closing (the old
     * `pendingFd = null`) leaked one descriptor per attach/detach cycle in
     * which streaming never started.
     */
    private fun closeUnclaimedFd() {
        val fd = pendingFd ?: return
        pendingFd = null
        try {
            android.os.ParcelFileDescriptor.adoptFd(fd).close()
            android.util.Log.i("MirrorUSB", "Closed unclaimed accessory FD=$fd")
        } catch (e: Exception) {
            android.util.Log.w("MirrorUSB", "Failed to close unclaimed FD=$fd: ${e.message}")
        }
    }

    private fun handleUsbAttachment(intent: Intent?) {
        val usbManager = getSystemService(USB_SERVICE) as UsbManager

        @Suppress("DEPRECATION")
        val accessory: UsbAccessory? = intent?.getParcelableExtra(UsbManager.EXTRA_ACCESSORY)

        if (accessory != null) {
            // A previous descriptor nobody claimed would otherwise be orphaned
            // by the assignment below.
            closeUnclaimedFd()

            val pfd = usbManager.openAccessory(accessory)
            if (pfd != null) {
                // Transfer fd ownership to Rust: after detachFd() the Java
                // side must never close it — the Rust USB loop closes it when
                // the session ends. (Closing it from both sides double-closed
                // an fd number that could already be reused by another thread.)
                val fd = pfd.detachFd()
                if (fd >= 0) {
                    android.util.Log.i("MirrorUSB", "Accessory opened: FD=$fd (${accessory.manufacturer} ${accessory.model})")
                    pendingFd = fd
                    // Ownership moves to Dart (and from there to Rust) only
                    // once the call is acknowledged. If Dart is not up yet the
                    // callback never succeeds, pendingFd stays set, and
                    // getInitialAccessory hands it over instead.
                    methodChannel?.invokeMethod("onUsbAttached", fd, object : MethodChannel.Result {
                        override fun success(result: Any?) {
                            if (pendingFd == fd) pendingFd = null
                        }
                        override fun error(code: String, msg: String?, details: Any?) {
                            android.util.Log.w("MirrorUSB", "onUsbAttached failed: $code $msg")
                        }
                        override fun notImplemented() {
                            android.util.Log.w("MirrorUSB", "onUsbAttached not implemented in Dart")
                        }
                    })
                } else {
                    android.util.Log.e("MirrorUSB", "Failed to detach accessory FD")
                }
            } else {
                android.util.Log.e("MirrorUSB", "Failed to open accessory")
            }
        } else {
            android.util.Log.w("MirrorUSB", "USB_ACCESSORY_ATTACHED but no accessory in extras")
        }
    }

    private fun handleUsbDetachment() {
        // Notify Flutter that the USB accessory was unplugged. If Dart claimed
        // the fd, the Rust read loop owns it and closes it itself.
        methodChannel?.invokeMethod("onUsbDetached", null)

        // Stop the mirroring service
        try {
            val serviceIntent = Intent(this, com.mirror.stream_mobile_app.service.MirrorForegroundService::class.java)
            stopService(serviceIntent)
        } catch (e: Exception) {
            android.util.Log.w("MirrorUSB", "Error stopping service on detach: ${e.message}")
        }

        closeUnclaimedFd()
    }

    private fun requestMediaProjection() {
        val mpManager = getSystemService(MEDIA_PROJECTION_SERVICE) as MediaProjectionManager
        @Suppress("DEPRECATION")
        startActivityForResult(mpManager.createScreenCaptureIntent(), REQUEST_MEDIA_PROJECTION)
    }

    @Suppress("DEPRECATION")
    override fun onActivityResult(requestCode: Int, resultCode: Int, data: Intent?) {
        super.onActivityResult(requestCode, resultCode, data)
        if (requestCode != REQUEST_MEDIA_PROJECTION) return

        if (resultCode != Activity.RESULT_OK || data == null) {
            // The user dismissed or denied the capture consent dialog. Dart
            // used to assume success and show "Streaming" regardless.
            android.util.Log.i("MirrorUSB", "Screen capture consent denied")
            methodChannel?.invokeMethod("onProjectionDenied", null)
            return
        }

        run {
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
            // ContextCompat picks startForegroundService on API 26+ and the
            // plain startService below that — a direct call would crash on
            // the API 24/25 devices this app still supports.
            ContextCompat.startForegroundService(this, serviceIntent)
            android.util.Log.i("MirrorUSB", "Foreground mirroring service started")
            methodChannel?.invokeMethod("onProjectionStarted", null)
        }
    }

    override fun onDestroy() {
        try {
            unregisterReceiver(usbDetachReceiver)
        } catch (_: Exception) {
        }
        super.onDestroy()
    }
}
