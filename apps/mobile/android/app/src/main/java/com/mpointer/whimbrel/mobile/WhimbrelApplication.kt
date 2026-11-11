package com.mpointer.whimbrel.mobile

import android.app.Activity
import android.app.Application
import android.content.Intent
import android.os.Build
import android.provider.Settings
import androidx.lifecycle.DefaultLifecycleObserver
import androidx.lifecycle.LifecycleOwner
import androidx.lifecycle.ProcessLifecycleOwner
import com.mpointer.whimbrel.mobile.auth.EnrollmentConfig
import com.mpointer.whimbrel.mobile.auth.GoogleSignIn
import com.mpointer.whimbrel.mobile.auth.LiveEnrollmentBackend
import com.mpointer.whimbrel.mobile.core.CoreMemoryStore
import com.mpointer.whimbrel.mobile.core.CoreSettingsPuller
import com.mpointer.whimbrel.mobile.core.CoreStore
import com.mpointer.whimbrel.mobile.core.CpuFeatures
import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.core.Native
import com.mpointer.whimbrel.mobile.core.SyncWorkerClient
import com.mpointer.whimbrel.mobile.reminders.AlarmReminderScheduler
import com.mpointer.whimbrel.mobile.reminders.PlanCache
import com.mpointer.whimbrel.mobile.reminders.ReminderCoordinator
import com.mpointer.whimbrel.mobile.reminders.ReminderReceiver
import com.mpointer.whimbrel.mobile.reminders.ReminderRefreshWorker
import com.mpointer.whimbrel.mobile.reminders.ReminderSettingsStore
import com.mpointer.whimbrel.mobile.reminders.ResourceReminderText
import com.mpointer.whimbrel.mobile.reminders.SystemAlarmBackend
import com.mpointer.whimbrel.mobile.secrets.KeystoreCipher
import com.mpointer.whimbrel.mobile.secrets.SecretVault
import com.mpointer.whimbrel.mobile.secrets.SyncSecrets
import com.mpointer.whimbrel.mobile.sync.CatchUpService
import com.mpointer.whimbrel.mobile.sync.EnrollmentBackend
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import com.mpointer.whimbrel.mobile.sync.PhoneKeyStore
import com.mpointer.whimbrel.mobile.sync.PhoneSettingsSync
import com.mpointer.whimbrel.mobile.sync.SharedPrefStore
import com.mpointer.whimbrel.mobile.sync.StorePaths
import com.mpointer.whimbrel.mobile.sync.StoreRemoval
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.sync.deleteStoreDir
import com.mpointer.whimbrel.mobile.sync.openStoreGuarded
import com.mpointer.whimbrel.mobile.sync.wipeHandler
import com.mpointer.whimbrel.mobile.theme.ThemeLoader
import com.mpointer.whimbrel.mobile.theme.ThemeStore
import java.io.File
import java.util.concurrent.Executors
import kotlinx.coroutines.CancellationException
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.MainScope
import kotlinx.coroutines.asCoroutineDispatcher
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch
import kotlinx.serialization.json.Json
import kotlinx.serialization.json.jsonObject
import kotlinx.serialization.json.jsonPrimitive

/**
 * Owns the process-wide singletons. Everything that touches [Native] hangs off `by lazy` and is read only
 * when [coreSupported], so a phone without FP16 never calls `System.loadLibrary`.
 */
class WhimbrelApplication : Application() {
    /** Main-dispatcher scope: [EnrollmentModel] is not thread-safe and runs its poll here. */
    val appScope: CoroutineScope = MainScope()

    var coreSupported = false
        private set

    /** The started MainActivity, for Credential Manager's account chooser; null in the background. */
    var activity: Activity? = null

    private var resumed = false

    /** [CatchUpService] is running (or asked to start): leaving the screen doesn't stop sync. */
    var catchUpRunning = false

    /** The notification permission is asked at most once per process, when catch-up first starts. */
    var askedForNotifications = false

    /** Starts the background catch-up (#572); call only from the foreground. */
    fun startCatchUp() {
        if (catchUpRunning) return
        // ForegroundServiceStartNotAllowedException is an IllegalStateException: the app just syncs on screen then.
        catchUpRunning = runCatching { startForegroundService(Intent(this, CatchUpService::class.java)) }.isSuccess
    }

    private val vault: SecretVault by lazy { SecretVault(File(noBackupFilesDir, "secrets"), KeystoreCipher()) }
    private val keys by lazy { PhoneKeyStore(vault) }
    private val rawPrefs by lazy { getSharedPreferences("whimbrel", MODE_PRIVATE) }
    private val prefs by lazy { SharedPrefStore(rawPrefs) }
    private val storeDir get() = StorePaths.storeFile(noBackupFilesDir).parentFile!!

    /** The themes; works on every phone, FP16 or not, and never touches [Native]. */
    val themes: ThemeStore by lazy { ThemeStore(prefs) }

    private val liveEnrollment: EnrollmentModel by lazy {
        val secrets = SyncSecrets(vault)
        val worker = SyncWorkerClient(Native, BuildConfig.SYNC_WORKER_BASE_URL)
        val configError = EnrollmentConfig.configErrorRes(BuildConfig.GOOGLE_WEB_CLIENT_ID, BuildConfig.SYNC_WORKER_BASE_URL)
        val backend = if (configError != null) null else {
            LiveEnrollmentBackend(GoogleSignIn.create(this, BuildConfig.GOOGLE_WEB_CLIENT_ID, Native), worker) { activity }
        }
        EnrollmentModel(
            backend = backend,
            secrets = secrets,
            native = Native,
            isPermanentError = worker::isPermanent,
            scope = appScope,
            deviceName = ::deviceName,
            clock = System::currentTimeMillis,
            sleep = { seconds -> delay(seconds * 1000) },
            configError = configError?.let(::getString),
        ).also {
            it.onWipe = {
                wipeHandler(prefs, rawPrefs, keys, liveSync, storeDir)()
                // Revoked or signed out: no reminder may outlive the account's data.
                reminders.clearOnWipe()
            }
        }
    }

    private val liveSync: SyncModel by lazy {
        val phoneSettings = PhoneSettingsSync(CoreSettingsPuller(Native), prefs, keys)
        SyncModel(
            openStore = {
                openStoreGuarded(rawPrefs, storeDir) { CoreStore.open(StorePaths.storeFile(noBackupFilesDir))?.let { CoreMemoryStore(it, Native) } }
            },
            credentials = enrollment,
            scope = appScope,
            // One thread: serializes the store handle and orders a reset's close and delete before any re-open.
            storeDispatcher = Executors.newSingleThreadExecutor { Thread(it, "store") }.asCoroutineDispatcher(),
            sleep = { seconds -> delay(seconds * 1000) },
            clock = System::currentTimeMillis,
            settingsPull = { c, epoch -> phoneSettings.pullOnce(c, epoch) },
            wipeEpoch = { PhoneSettingsSync.currentWipeEpoch },
        )
    }

    private val reminderPrefs by lazy { getSharedPreferences("reminders", MODE_PRIVATE) }

    /**
     * Reminders: planning, the plan check and scheduling. Only ever read when [coreSupported]. Its models are
     * looked up on use, so a demo started later is the one it sees (and the demo turns it off).
     */
    val reminders: ReminderCoordinator by lazy {
        ReminderCoordinator.create(
            sync = { sync },
            enrollment = { enrollment },
            scheduler = AlarmReminderScheduler(reminderPrefs, SystemAlarmBackend(this)),
            settings = ReminderSettingsStore(reminderPrefs),
            plan = PlanCache(reminderPrefs),
            text = ResourceReminderText(this),
            requestPermission = { (activity as? MainActivity)?.requestNotificationPermission() ?: false },
        )
    }

    /** The demo's models when a debug launch asked for one; replaces the live ones for the rest of the process. */
    private var demo: Pair<EnrollmentModel, SyncModel>? = null

    val enrollment: EnrollmentModel get() = demo?.first ?: liveEnrollment

    /** The content sync; only ever read when [coreSupported]. */
    val sync: SyncModel get() = demo?.second ?: liveSync

    /**
     * Debug demo (see DemoHooks): an approved enrollment with throwaway keys in the cache directory and a sync
     * over [store], so no real state, Google or server is touched. Call before any UI reads [enrollment] or [sync];
     * a second call (an Activity restart) keeps the demo already running.
     */
    internal fun startDemo(store: MeetingSyncStore, backend: EnrollmentBackend) {
        if (!coreSupported || demo != null) return
        // The demo never checks the plan, schedules or asks for permission, whatever the cached plan says.
        reminders.isEnabled = false
        val secrets = SyncSecrets(SecretVault(File(cacheDir, "demo-secrets").also { it.deleteRecursively() }, KeystoreCipher()))
        // Keys already held: the model starts approved and its silent resume lands the (fake) token.
        secrets.deviceSecretKey = Json.parseToJsonElement(Native.syncGenerateDeviceKeypair()!!).jsonObject.getValue("secret_key_base64").jsonPrimitive.content
        secrets.groupKey = "demo"
        val enrollment = EnrollmentModel(
            backend = backend,
            secrets = secrets,
            native = Native,
            isPermanentError = { false },
            scope = appScope,
            deviceName = ::deviceName,
            clock = System::currentTimeMillis,
            sleep = { seconds -> delay(seconds * 1000) },
        )
        val sync = SyncModel(
            openStore = { store },
            credentials = enrollment,
            scope = appScope,
            storeDispatcher = Executors.newSingleThreadExecutor { Thread(it, "store") }.asCoroutineDispatcher(),
            sleep = { seconds -> delay(seconds * 1000) },
            clock = System::currentTimeMillis,
        )
        demo = enrollment to sync
    }

    override fun onCreate() {
        super.onCreate()
        themes.install(
            ThemeLoader.loadDirectory(
                assets.list("").orEmpty().filter { it.endsWith(".json") }.map { name -> name to assets.open(name).use { String(it.readBytes()) } },
            ),
        )
        coreSupported = CpuFeatures.hasFp16(runCatching { File("/proc/cpuinfo").readText() }.getOrDefault(""))
        if (!coreSupported) return
        ReminderReceiver.ensureChannel(this)
        ReminderRefreshWorker.enqueuePeriodic(this)
        // A wipe cut short (killed, delete failed) finishes now, before anything opens the store.
        StoreRemoval.finishIfPending(rawPrefs) { deleteStoreDir(storeDir) }
        // No polling in the background; coming back re-checks a pending approval at once.
        ProcessLifecycleOwner.get().lifecycle.addObserver(object : DefaultLifecycleObserver {
            override fun onStart(owner: LifecycleOwner) {
                appScope.launch { enrollment.enterForeground() }
            }

            override fun onStop(owner: LifecycleOwner) = enrollment.enterBackground()
        })
    }

    /** The silent re-check on launch, once per process (an Activity restart must not repeat it). */
    fun resumeEnrollmentOnce() {
        if (resumed) return
        resumed = true
        appScope.launch { enrollment.launch() }
        // The plan check at launch, then after every sync: a throttled check and a fresh set of reminders.
        appScope.launch { reminderStep { reminders.refreshPlanIfDue(force = true) } }
        appScope.launch {
            sync.readRevision.collect {
                reminderStep { reminders.refreshPlanIfDue(force = false) }
                reminderStep { reminders.reschedule() }
            }
        }
    }

    /** A reminder step must never take the app down (this scope has no handler); a cancellation still propagates. */
    private suspend fun reminderStep(step: suspend () -> Unit) {
        try {
            step()
        } catch (e: CancellationException) {
            throw e
        } catch (e: Exception) {
            android.util.Log.w("reminders", "step failed: ${e.javaClass.simpleName}")
        }
    }

    /** What the Mac's Devices list shows for this phone: the name the owner gave it, else the model. */
    fun deviceName(): String =
        runCatching { Settings.Global.getString(contentResolver, "device_name") }.getOrNull()?.trim()?.takeIf { it.isNotEmpty() }
            ?: Build.MODEL.trim().takeIf { it.isNotEmpty() }
            ?: getString(R.string.device_name_fallback)
}
