package com.mpointer.whimbrel.mobile.reminders

import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent
import com.mpointer.whimbrel.mobile.MainActivity
import com.mpointer.whimbrel.mobile.R

/** Posts a reminder when its alarm fires. Tapping it opens the app on Today (Android has no routes). */
class ReminderReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        val title = intent.getStringExtra(EXTRA_TITLE) ?: return
        val body = intent.getStringExtra(EXTRA_BODY)
        ensureChannel(context)
        val open = PendingIntent.getActivity(
            context,
            0,
            Intent(context, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_UPDATE_CURRENT or PendingIntent.FLAG_IMMUTABLE,
        )
        val builder = Notification.Builder(context, CHANNEL_ID)
            .setSmallIcon(android.R.drawable.ic_popup_reminder)
            .setContentTitle(title)
            .setContentIntent(open)
            .setAutoCancel(true)
            .setCategory(Notification.CATEGORY_REMINDER)
        if (!body.isNullOrEmpty()) builder.setContentText(body).setStyle(Notification.BigTextStyle().bigText(body))
        // One notification per reminder: a repeat of the same alarm replaces it. Without the permission this is dropped.
        val id = intent.data?.schemeSpecificPart?.let(ReminderRequestCodes::codeFor) ?: title.hashCode()
        context.getSystemService(NotificationManager::class.java).notify(id, builder.build())
    }

    companion object {
        const val ACTION = "com.mpointer.whimbrel.mobile.REMINDER"
        const val EXTRA_TITLE = "title"
        const val EXTRA_BODY = "body"
        const val CHANNEL_ID = "reminders"

        /** Default importance; created at app start and again here, so a reminder never posts without it. */
        fun ensureChannel(context: Context) {
            context.getSystemService(NotificationManager::class.java)
                .createNotificationChannel(NotificationChannel(CHANNEL_ID, context.getString(R.string.reminders_channel_name), NotificationManager.IMPORTANCE_DEFAULT))
        }
    }
}
