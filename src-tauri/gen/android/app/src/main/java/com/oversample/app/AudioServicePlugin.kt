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

    private var pendingNotifInvoke: Invoke? = null
    private var pendingStartMode: String = "recording"

    companion object {
        const val REQUEST_POST_NOTIFICATIONS = 1101
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

        // On API 33+, request POST_NOTIFICATIONS so the ongoing FGS notification
        // is visible. The service still runs if denied, so we start it either way.
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU &&
            ContextCompat.checkSelfPermission(activity, Manifest.permission.POST_NOTIFICATIONS)
            != PackageManager.PERMISSION_GRANTED) {
            pendingNotifInvoke = invoke
            pendingStartMode = args.mode
            ActivityCompat.requestPermissions(
                activity,
                arrayOf(Manifest.permission.POST_NOTIFICATIONS),
                REQUEST_POST_NOTIFICATIONS,
            )
            return
        }

        ForegroundAudioService.start(activity, args.mode)
        val result = JSObject()
        result.put("started", true)
        invoke.resolve(result)
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
     *  Starts the service regardless of the notification-permission outcome. */
    fun handlePermissionResult(requestCode: Int, grantResults: IntArray) {
        if (requestCode != REQUEST_POST_NOTIFICATIONS) return
        val invoke = pendingNotifInvoke ?: return
        pendingNotifInvoke = null

        ForegroundAudioService.start(activity, pendingStartMode)
        val granted = grantResults.isNotEmpty() &&
            grantResults[0] == PackageManager.PERMISSION_GRANTED
        val result = JSObject()
        result.put("started", true)
        result.put("notificationsGranted", granted)
        invoke.resolve(result)
    }
}
