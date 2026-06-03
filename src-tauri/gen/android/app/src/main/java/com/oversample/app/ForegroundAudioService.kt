// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Foreground service that keeps live microphone monitoring and recording
// running while the app is backgrounded. The actual audio capture runs in the
// Rust process (cpal/Oboe on a native thread) and the WAV writer streams to
// disk there; this service's job is to (a) make that legal under Android 14/15
// background-execution limits via a `microphone` foreground service, and
// (b) hold a partial wake lock so the CPU keeps servicing the audio callback
// during Doze. Started/stopped from AudioServicePlugin on the Listen/Record tap.

package com.oversample.app

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.util.Log
import androidx.core.app.NotificationCompat

private const val TAG = "ForegroundAudioService"
private const val CHANNEL_ID = "oversample_audio_capture"
private const val NOTIFICATION_ID = 0x0A11
// Backstop so a wake lock can never leak indefinitely if the service is torn
// down abnormally; normal stop releases it well before this.
private const val WAKELOCK_TIMEOUT_MS = 6L * 60L * 60L * 1000L // 6 hours

class ForegroundAudioService : Service() {

    companion object {
        const val ACTION_START = "com.oversample.app.action.START_AUDIO"
        const val ACTION_STOP = "com.oversample.app.action.STOP_AUDIO"
        const val EXTRA_MODE = "mode" // "listening" | "recording"

        /** Start (or update the notification of) the foreground audio service.
         *  MUST be called while the Activity is foreground — Android 14+ forbids
         *  starting a `microphone` FGS from the background. */
        fun start(ctx: Context, mode: String) {
            val i = Intent(ctx, ForegroundAudioService::class.java).apply {
                action = ACTION_START
                putExtra(EXTRA_MODE, mode)
            }
            // startForegroundService is required on O+; the service must then
            // call startForeground() within ~5 s or the system ANRs it.
            if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) ctx.startForegroundService(i)
            else ctx.startService(i)
        }

        /** Stop the service (idempotent). */
        fun stop(ctx: Context) {
            try {
                ctx.startService(Intent(ctx, ForegroundAudioService::class.java).apply {
                    action = ACTION_STOP
                })
            } catch (e: Exception) {
                // startService can throw if the app is backgrounded and the
                // service isn't running; nothing to stop in that case.
                Log.w(TAG, "stop: ${e.message}")
            }
        }
    }

    private var wakeLock: PowerManager.WakeLock? = null

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        when (intent?.action) {
            ACTION_STOP -> {
                stopSelfCleanly()
                return START_NOT_STICKY
            }
            else -> {
                val mode = intent?.getStringExtra(EXTRA_MODE) ?: "recording"
                createChannel()
                val notification = buildNotification(mode)
                try {
                    if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) {
                        startForeground(
                            NOTIFICATION_ID, notification,
                            ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE,
                        )
                    } else {
                        startForeground(NOTIFICATION_ID, notification)
                    }
                    acquireWakeLock()
                } catch (e: Exception) {
                    // e.g. SecurityException if RECORD_AUDIO was revoked mid-flight,
                    // or ForegroundServiceStartNotAllowedException from background.
                    Log.e(TAG, "startForeground failed: ${e.message}")
                    stopSelfCleanly()
                }
            }
        }
        // Deliberately NOT sticky: a null-intent restart must never try to
        // re-enter a mic FGS from the background (illegal on Android 14+).
        return START_NOT_STICKY
    }

    private fun buildNotification(mode: String): Notification {
        val label = if (mode == "listening") "Listening (live)" else "Recording audio"
        // Tap → resurface MainActivity (singleTask, so it just comes forward).
        val openIntent = Intent(this, MainActivity::class.java).apply {
            flags = Intent.FLAG_ACTIVITY_SINGLE_TOP or Intent.FLAG_ACTIVITY_NEW_TASK
        }
        val pi = PendingIntent.getActivity(
            this, 0, openIntent, PendingIntent.FLAG_IMMUTABLE,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_btn_speak_now)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(label)
            .setOngoing(true)
            .setContentIntent(pi)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }

    private fun createChannel() {
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            val mgr = getSystemService(NotificationManager::class.java)
            if (mgr.getNotificationChannel(CHANNEL_ID) == null) {
                val channel = NotificationChannel(
                    CHANNEL_ID, "Audio capture",
                    NotificationManager.IMPORTANCE_LOW, // no sound / heads-up
                ).apply { setShowBadge(false) }
                mgr.createNotificationChannel(channel)
            }
        }
    }

    private fun acquireWakeLock() {
        if (wakeLock?.isHeld == true) return
        val pm = getSystemService(Context.POWER_SERVICE) as PowerManager
        wakeLock = pm.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "oversample:AudioCapture").apply {
            setReferenceCounted(false)
            acquire(WAKELOCK_TIMEOUT_MS)
        }
    }

    private fun releaseWakeLock() {
        try {
            if (wakeLock?.isHeld == true) wakeLock?.release()
        } catch (e: Exception) {
            Log.w(TAG, "releaseWakeLock: ${e.message}")
        }
        wakeLock = null
    }

    private fun stopSelfCleanly() {
        releaseWakeLock()
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.N) {
            stopForeground(STOP_FOREGROUND_REMOVE)
        } else {
            @Suppress("DEPRECATION")
            stopForeground(true)
        }
        stopSelf()
    }

    override fun onDestroy() {
        // Safety net so the wake lock can never outlive the process.
        releaseWakeLock()
        super.onDestroy()
    }
}
