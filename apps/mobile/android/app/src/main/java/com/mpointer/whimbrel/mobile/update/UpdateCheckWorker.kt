package com.mpointer.whimbrel.mobile.update

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.Context
import android.content.Intent
import androidx.core.app.NotificationManagerCompat
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.NetworkType
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.mpointer.whimbrel.mobile.BuildConfig
import com.mpointer.whimbrel.mobile.MainActivity
import com.mpointer.whimbrel.mobile.R
import java.util.concurrent.TimeUnit

/**
 * The background update check: every 6 hours with a network, it asks for the newest release (UpdateModel limits the
 * request itself to one per 6 hours) and posts one notification per new version. Tapping it opens the app, where the
 * "Update available" banner has the download. A version dismissed with Not now is never announced. Without the
 * notification permission nothing is posted; the banner still shows when the app is opened.
 */
class UpdateCheckWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val context = applicationContext
        val model = UpdateModel(context.getSharedPreferences(UpdateModel.PREFS, Context.MODE_PRIVATE), BuildConfig.VERSION_NAME)
        model.checkIfDue()
        if (!NotificationManagerCompat.from(context).areNotificationsEnabled()) return Result.success()
        model.takeNotification()?.let { post(context, it) }
        return Result.success()
    }

    companion object {
        const val PERIODIC_NAME = "update-check"
        const val CHANNEL_ID = "updates"
        private const val NOTIFICATION_ID = 0x5550_4454 // one update notification at a time; a newer version replaces it

        /** Every 6 hours with a network; KEEP leaves a running schedule alone. */
        fun enqueuePeriodic(context: Context) {
            val network = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()
            val request = PeriodicWorkRequestBuilder<UpdateCheckWorker>(6, TimeUnit.HOURS).setConstraints(network).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(PERIODIC_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        fun ensureChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java)
                .createNotificationChannel(NotificationChannel(CHANNEL_ID, context.getString(R.string.update_channel_name), NotificationManager.IMPORTANCE_DEFAULT))
        }

        private fun post(context: Context, update: AvailableUpdate) {
            ensureChannel(context)
            val open = PendingIntent.getActivity(
                context,
                NOTIFICATION_ID,
                Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
                PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
            )
            val notification = Notification.Builder(context, CHANNEL_ID)
                .setSmallIcon(android.R.drawable.stat_sys_download_done)
                .setContentTitle(context.getString(R.string.update_available, update.version))
                .setContentText(context.getString(R.string.update_notification_text))
                .setContentIntent(open)
                .setAutoCancel(true)
                .build()
            context.getSystemService(NotificationManager::class.java).notify(NOTIFICATION_ID, notification)
        }
    }
}
