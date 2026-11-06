package com.mpointer.whimbrel.mobile.sync

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Intent
import android.content.pm.ServiceInfo
import android.os.IBinder
import com.mpointer.whimbrel.mobile.MainActivity
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.WhimbrelApplication
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.cancel
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch

/**
 * Keeps the app's own [SyncModel] loop running while the phone catches up (#572), so pulling goes on when the app leaves
 * the screen. Holds no store of its own: the loop, its store thread and its handle are the app's. Started from the
 * foreground by MainActivity; ends itself once a round reports nothing more to pull, on a failure (backoff), on
 * sign-out, revoke or reset, or after [CatchUp.maxRunMs]. Ending in the background stops the loop.
 */
class CatchUpService : Service() {
    private val scope = MainScope()
    private val app get() = application as WhimbrelApplication

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        try {
            startForeground(NOTIFICATION_ID, notification(), ServiceInfo.FOREGROUND_SERVICE_TYPE_DATA_SYNC)
        } catch (_: IllegalStateException) {
            // Refused (e.g. the day's dataSync time is used up): no catch-up this time; the app still syncs on screen.
            stopSelf()
            return START_NOT_STICKY
        }
        if (!app.coreSupported) {
            finish()
            return START_NOT_STICKY
        }
        val startedAtMs = System.currentTimeMillis()
        scope.launch {
            combine(app.enrollment.status, app.sync.catchingUp) { status, catchingUp ->
                CatchUp.shouldKeepRunning(status == EnrollmentStatus.Approved, catchingUp, startedAtMs, System.currentTimeMillis())
            }.collect { keep -> if (!keep) finish() }
        }
        scope.launch {
            delay(CatchUp.maxRunMs)
            finish()
        }
        return START_NOT_STICKY
    }

    /** Android 15's own dataSync limit. */
    override fun onTimeout(startId: Int, fgsType: Int) = finish()

    override fun onDestroy() {
        scope.cancel()
        app.catchUpRunning = false
        // Off screen nothing else keeps the loop: stop it (MainActivity starts it again on return).
        if (app.activity == null && app.coreSupported) app.sync.stop()
        super.onDestroy()
    }

    private fun finish() {
        stopForeground(STOP_FOREGROUND_REMOVE)
        stopSelf()
    }

    private fun notification(): Notification {
        val manager = getSystemService(NotificationManager::class.java)
        manager.createNotificationChannel(NotificationChannel(CHANNEL_ID, getString(R.string.catch_up_channel), NotificationManager.IMPORTANCE_LOW))
        val open = PendingIntent.getActivity(this, 0, Intent(this, MainActivity::class.java), PendingIntent.FLAG_IMMUTABLE)
        return Notification.Builder(this, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.stat_notify_sync)
            .setContentTitle(getString(R.string.catch_up_notification))
            .setContentIntent(open)
            .setOngoing(true)
            .build()
    }

    private companion object {
        const val CHANNEL_ID = "catch_up"
        const val NOTIFICATION_ID = 572
    }
}
