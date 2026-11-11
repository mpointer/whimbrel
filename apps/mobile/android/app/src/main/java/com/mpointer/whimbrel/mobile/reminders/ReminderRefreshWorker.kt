package com.mpointer.whimbrel.mobile.reminders

import android.content.Context
import androidx.work.Constraints
import androidx.work.CoroutineWorker
import androidx.work.ExistingPeriodicWorkPolicy
import androidx.work.ExistingWorkPolicy
import androidx.work.NetworkType
import androidx.work.OneTimeWorkRequestBuilder
import androidx.work.PeriodicWorkRequestBuilder
import androidx.work.WorkManager
import androidx.work.WorkerParameters
import com.mpointer.whimbrel.mobile.WhimbrelApplication
import com.mpointer.whimbrel.mobile.sync.EnrollmentStatus
import java.util.concurrent.TimeUnit
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.withContext

/**
 * The background refresh: syncs once, checks the plan if due and reschedules, so reminders follow new
 * meetings while the app is closed. The app's models are main-thread, so the work runs on Main.
 */
class ReminderRefreshWorker(context: Context, params: WorkerParameters) : CoroutineWorker(context, params) {
    override suspend fun doWork(): Result {
        val app = applicationContext as? WhimbrelApplication ?: return Result.success()
        // Not enrolled here, or a phone without the core: nothing to refresh.
        if (!app.coreSupported) return Result.success()
        val ok = withContext(Dispatchers.Main.immediate) {
            if (app.enrollment.status.value != EnrollmentStatus.Approved) true
            else ReminderRefresh.run(app.reminders) { app.sync.syncNow() }
        }
        return if (ok) Result.success() else Result.retry()
    }

    companion object {
        const val PERIODIC_NAME = "reminders-refresh"
        const val ONCE_NAME = "reminders-refresh-once"
        private val network = Constraints.Builder().setRequiredNetworkType(NetworkType.CONNECTED).build()

        /** Every 30 minutes with a network (WorkManager's minimum is 15); KEEP leaves a running schedule alone. */
        fun enqueuePeriodic(context: Context) {
            val request = PeriodicWorkRequestBuilder<ReminderRefreshWorker>(30, TimeUnit.MINUTES).setConstraints(network).build()
            WorkManager.getInstance(context).enqueueUniquePeriodicWork(PERIODIC_NAME, ExistingPeriodicWorkPolicy.KEEP, request)
        }

        /** After a reboot or an update: one run now (alarms are gone), without waiting for the periodic one. */
        fun enqueueOnce(context: Context) {
            // No network constraint: rescheduling reads the replica, and a failed sync does not stop it.
            val request = OneTimeWorkRequestBuilder<ReminderRefreshWorker>().build()
            WorkManager.getInstance(context).enqueueUniqueWork(ONCE_NAME, ExistingWorkPolicy.REPLACE, request)
        }
    }
}
