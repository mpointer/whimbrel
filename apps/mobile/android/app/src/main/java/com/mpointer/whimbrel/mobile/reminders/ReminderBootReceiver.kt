package com.mpointer.whimbrel.mobile.reminders

import android.content.BroadcastReceiver
import android.content.Context
import android.content.Intent

/** A reboot or an app update clears every alarm: queue one refresh, which reschedules them. */
class ReminderBootReceiver : BroadcastReceiver() {
    override fun onReceive(context: Context, intent: Intent) {
        when (intent.action) {
            Intent.ACTION_BOOT_COMPLETED, Intent.ACTION_MY_PACKAGE_REPLACED -> ReminderRefreshWorker.enqueueOnce(context)
        }
    }
}
