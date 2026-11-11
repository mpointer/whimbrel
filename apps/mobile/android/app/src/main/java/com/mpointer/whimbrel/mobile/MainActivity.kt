package com.mpointer.whimbrel.mobile

import android.Manifest
import android.content.pm.PackageManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import android.content.Intent
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.runtime.collectAsState
import androidx.lifecycle.lifecycleScope
import kotlinx.coroutines.Job
import kotlinx.coroutines.flow.combine
import kotlinx.coroutines.launch
import androidx.compose.runtime.getValue
import androidx.compose.foundation.layout.padding
import androidx.compose.ui.Modifier
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.sync.CatchUp
import com.mpointer.whimbrel.mobile.sync.EnrollmentStatus
import com.mpointer.whimbrel.mobile.ui.EnrollmentScreen
import com.mpointer.whimbrel.mobile.theme.WhimbrelTheme
import com.mpointer.whimbrel.mobile.ui.RootScreen

class MainActivity : ComponentActivity() {
    private var syncGate: Job? = null
    // Debug demo launches can arrive in a running Activity (onNewIntent); the shell applies each one.
    private var start by mutableStateOf(DemoHooks.start(Intent()))
    private val app get() = application as WhimbrelApplication

    override fun onCreate(savedInstanceState: Bundle?) {
        super.onCreate(savedInstanceState)
        enableEdgeToEdge()
        // Debug demo launches swap in their models before anything reads them; a no-op in release.
        DemoHooks.apply(app, intent)
        start = DemoHooks.start(intent)
        if (!app.coreSupported) {
            // Never touch Native on this phone: the message and nothing else.
            setContent {
                WhimbrelTheme(app.themes) {
                    Surface { Text(stringResource(R.string.unsupported_cpu), Modifier.safeDrawingPadding().padding(24.dp)) }
                }
            }
            return
        }
        val enrollment = app.enrollment
        app.resumeEnrollmentOnce()
        setContent {
            WhimbrelTheme(app.themes) {
                val status by enrollment.status.collectAsState()
                if (status == EnrollmentStatus.Approved) {
                    RootScreen(app.sync, enrollment, app.reminders, app.themes, app.deviceName(), start)
                } else {
                    EnrollmentScreen(enrollment)
                }
            }
        }
    }

    override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        DemoHooks.apply(app, intent)
        start = DemoHooks.start(intent).let { it.copy(launchId = start.launchId + 1) }
    }

    override fun onStart() {
        super.onStart()
        app.activity = this
        if (app.coreSupported) {
            // The reminders' permission prompt waits for this; Cloud turning on while away is asked now.
            app.reminders.isForeground = true
            // Sync runs while the app is open and approved; a revoke or sign-out stops it. While the phone is behind,
            // the catch-up service keeps it running after the app leaves the screen (#572).
            syncGate = lifecycleScope.launch {
                combine(app.enrollment.status, app.sync.catchingUp) { status, catchingUp -> (status == EnrollmentStatus.Approved) to catchingUp }
                    .collect { (approved, catchingUp) ->
                        if (approved) app.sync.start() else app.sync.stop()
                        if (CatchUp.shouldStart(approved, catchingUp)) {
                            requestNotificationPermission()
                            app.startCatchUp()
                        }
                    }
            }
        }
    }

    /**
     * The catch-up notification and the reminders need it on Android 13+; denied, the catch-up service still runs
     * (the system lists it instead). The prompt shows at most once per process, whichever feature asks first.
     * True when the question is settled: the prompt was launched now, the permission is already granted, or it is not needed.
     * False when the prompt was suppressed because one already showed this process; reminders then try again next foreground.
     */
    fun requestNotificationPermission(): Boolean {
        if (Build.VERSION.SDK_INT < Build.VERSION_CODES.TIRAMISU) return true
        if (checkSelfPermission(Manifest.permission.POST_NOTIFICATIONS) == PackageManager.PERMISSION_GRANTED) return true
        // Already prompted this process (by whichever feature asked first): suppressed, so the question is not settled.
        if (app.askedForNotifications) return false
        app.askedForNotifications = true
        requestPermissions(arrayOf(Manifest.permission.POST_NOTIFICATIONS), 0)
        return true
    }

    override fun onStop() {
        syncGate?.cancel()
        syncGate = null
        if (app.coreSupported) app.reminders.isForeground = false
        if (app.activity === this) app.activity = null
        if (app.coreSupported && !app.catchUpRunning) app.sync.stop()
        super.onStop()
    }
}
