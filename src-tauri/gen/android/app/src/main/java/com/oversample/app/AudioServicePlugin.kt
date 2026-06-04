// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Tauri plugin that starts/stops the ForegroundAudioService and surfaces
// battery-optimization controls to the frontend. Mirrors the @TauriPlugin /
// @Command / handlePermissionResult conventions of UsbAudioPlugin.

package com.oversample.app

import android.Manifest
import android.app.Activity
import android.content.Context
import android.content.Intent
import android.content.pm.PackageManager
import android.os.Build
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import android.webkit.WebView
import androidx.core.app.ActivityCompat
import androidx.core.content.ContextCompat
import app.tauri.annotation.Command
import app.tauri.annotation.InvokeArg
import app.tauri.annotation.TauriPlugin
import app.tauri.plugin.Invoke
import app.tauri.plugin.JSObject
import app.tauri.plugin.Plugin

private const val TAG = "AudioServicePlugin"

@InvokeArg
data class StartAudioArgs(val mode: String = "recording") // "listening" | "recording"

@TauriPlugin
class AudioServicePlugin(private val activity: Activity) : Plugin(activity) {

    private var pendingNotifPermInvoke: Invoke? = null

    /** Receive the WebView reference (set from MainActivity.onWebViewCreate) so the
     *  notification "Stop" action can push a stop into the WASM frontend. */
    fun setWebView(webView: WebView) {
        webViewRef = webView
    }

    companion object {
        const val REQUEST_NOTIF_PERM = 1102

        // Held statically so ForegroundAudioService (a separate component) can
        // reach the frontend without a binding. Cleared implicitly when the
        // process dies, which is the only time it would be stale.
        private var webViewRef: WebView? = null

        /** Push a stop request into the WASM frontend (calls the global it exposes
         *  in app.rs). Runs on the WebView's thread. Returns false if no WebView is
         *  available, so the caller can fall back to stopping the service directly.
         *  Used a native push rather than a poll because the user usually taps the
         *  notification while the app is backgrounded, where JS timers are throttled
         *  but evaluateJavascript still executes. */
        fun dispatchUserStop(): Boolean {
            val wv = webViewRef ?: return false
            wv.post {
                wv.evaluateJavascript(
                    "window.__oversampleStopCapture && window.__oversampleStopCapture()",
                    null,
                )
            }
            return true
        }
    }

    /** Start the foreground audio service. Must be invoked while the Activity is
     *  foreground (the frontend calls this synchronously from the Listen/Record
     *  tap, never from a background event). */
    @Command
    fun startForegroundAudio(invoke: Invoke) {
        val args = invoke.parseArgs(StartAudioArgs::class.java)

        // RECORD_AUDIO must be granted before starting a microphone FGS, or
        // startForeground() throws. The frontend already requests it via
        // usb-audio|requestAudioPermission before opening the mic; this is a
        // belt-and-suspenders guard.
        if (ContextCompat.checkSelfPermission(activity, Manifest.permission.RECORD_AUDIO)
            != PackageManager.PERMISSION_GRANTED) {
            invoke.reject("RECORD_AUDIO not granted")
            return
        }

        // Notification permission (POST_NOTIFICATIONS) is requested separately
        // and proactively during mic setup (requestNotificationPermission), with
        // an in-app rationale — so we never trigger an OS prompt here at
        // Listen/Record time. The FGS runs regardless; if notifications are
        // denied the ongoing notification is simply suppressed by the OS.
        ForegroundAudioService.start(activity, args.mode)
        val result = JSObject()
        result.put("started", true)
        invoke.resolve(result)
    }

    /** Report whether the notification permission is granted, and whether a
     *  runtime request is even needed (API 33+). Lets the frontend decide
     *  whether to show the rationale before requesting. */
    @Command
    fun isNotificationPermissionGranted(invoke: Invoke) {
        val runtimeRequired = Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU
        val granted = !runtimeRequired ||
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED
        val result = JSObject()
        result.put("granted", granted)
        result.put("runtimeRequired", runtimeRequired)
        invoke.resolve(result)
    }

    /** Request POST_NOTIFICATIONS (API 33+). Resolves immediately as granted on
     *  older OSes or if already granted. Called after the in-app rationale so the
     *  user understands why the OS prompt appears. */
    @Command
    fun requestNotificationPermission(invoke: Invoke) {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU ||
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS) ==
                PackageManager.PERMISSION_GRANTED) {
            val result = JSObject()
            result.put("granted", true)
            invoke.resolve(result)
            return
        }
        pendingNotifPermInvoke = invoke
        ActivityCompat.requestPermissions(
            activity,
            arrayOf(Manifest.permission.POST_NOTIFICATIONS),
            REQUEST_NOTIF_PERM,
        )
    }

    /** Update the running service's notification mode (listening <-> recording).
     *  Idempotent: start() re-issues startForeground(), updating the notification. */
    @Command
    fun updateForegroundAudio(invoke: Invoke) {
        val args = invoke.parseArgs(StartAudioArgs::class.java)
        ForegroundAudioService.start(activity, args.mode)
        val result = JSObject()
        result.put("ok", true)
        invoke.resolve(result)
    }

    /** Stop the foreground audio service (idempotent). */
    @Command
    fun stopForegroundAudio(invoke: Invoke) {
        ForegroundAudioService.stop(activity)
        val result = JSObject()
        result.put("stopped", true)
        invoke.resolve(result)
    }

    /** Whether the app is currently exempt from battery optimization (Doze). */
    @Command
    fun isIgnoringBatteryOptimizations(invoke: Invoke) {
        val pm = activity.getSystemService(Context.POWER_SERVICE) as PowerManager
        val ignoring = pm.isIgnoringBatteryOptimizations(activity.packageName)
        val result = JSObject()
        result.put("ignoring", ignoring)
        invoke.resolve(result)
    }

    /** Open battery-optimization settings so the user can allow unrestricted
     *  background activity. Uses the no-permission settings deep link
     *  (Play-policy-safe), not ACTION_REQUEST_IGNORE_BATTERY_OPTIMIZATIONS. */
    @Command
    fun requestDisableBatteryOptimization(invoke: Invoke) {
        try {
            val intent = Intent(Settings.ACTION_IGNORE_BATTERY_OPTIMIZATION_SETTINGS).apply {
                flags = Intent.FLAG_ACTIVITY_NEW_TASK
            }
            activity.startActivity(intent)
            val result = JSObject()
            result.put("opened", true)
            invoke.resolve(result)
        } catch (e: Exception) {
            Log.w(TAG, "battery settings intent failed: ${e.message}")
            invoke.reject("Could not open battery settings: ${e.message}")
        }
    }

    /** Called from MainActivity.onRequestPermissionsResult (mirrors UsbAudioPlugin).
     *  Resolves the pending requestNotificationPermission call with the outcome. */
    fun handlePermissionResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != REQUEST_NOTIF_PERM) return
        val invoke = pendingNotifPermInvoke ?: return
        pendingNotifPermInvoke = null

        val granted = grantResults.isNotEmpty() &&
            grantResults[0] == PackageManager.PERMISSION_GRANTED
        val result = JSObject()
        result.put("granted", granted)
        invoke.resolve(result)
    }
}
