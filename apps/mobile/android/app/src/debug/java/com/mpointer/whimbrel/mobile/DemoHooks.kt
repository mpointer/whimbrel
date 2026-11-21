package com.mpointer.whimbrel.mobile

import android.content.Intent
import androidx.compose.runtime.Composable
import com.mpointer.whimbrel.mobile.design.WGallery
import com.mpointer.whimbrel.mobile.read.DemoAsk
import com.mpointer.whimbrel.mobile.read.DemoLimitBackend
import com.mpointer.whimbrel.mobile.read.DemoSyncStore
import com.mpointer.whimbrel.mobile.read.DemoSyncedBackend
import com.mpointer.whimbrel.mobile.ui.RootStart
import com.mpointer.whimbrel.mobile.ui.RootTab

/**
 * Debug launch extras for screenshots: `adb shell am start --ez whimbrel_demo_synced true --es whimbrel_demo_tab meetings
 * --el whimbrel_demo_meeting 3 --ez whimbrel_demo_settings true`; also `--es whimbrel_demo_tab ask|topics`,
 * `--es whimbrel_demo_topic acme|hiring|offsite` and `--ez whimbrel_demo_empty true` (no topics, no hosted key;
 * it only takes effect on the launch that starts the demo). Nothing here touches real state or reaches a provider.
 */
object DemoHooks {
    fun apply(app: WhimbrelApplication, intent: Intent) {
        // `whimbrel_demo_empty`: no topics and no hosted key, so the first-run Ask and Topics states can be shown.
        val empty = intent.getBooleanExtra("whimbrel_demo_empty", false)
        // `whimbrel_demo_limit`: a phone that is not approved and is told the free plan's phone slot is taken.
        if (intent.getBooleanExtra("whimbrel_demo_limit", false)) {
            app.startDemo(DemoSyncStore(), DemoLimitBackend(), approved = false)
            return
        }
        if (intent.getBooleanExtra("whimbrel_demo_synced", false)) {
            app.startDemo(DemoSyncStore(empty = empty), DemoSyncedBackend(pendingDevices = intent.getBooleanExtra("whimbrel_demo_devices", false)), ask = { scope -> DemoAsk.model(scope, empty) })
        }
    }

    /** The shell's first tab, meeting and sheet; a meeting implies the Meetings tab, as on the iPhone. */
    fun start(intent: Intent): RootStart {
        val meeting = intent.getLongExtra("whimbrel_demo_meeting", -1L).takeIf { it >= 0 }
        val tab = intent.getStringExtra("whimbrel_demo_tab")?.let { name -> RootTab.entries.firstOrNull { it.name.equals(name, ignoreCase = true) } }
        val topic = intent.getStringExtra("whimbrel_demo_topic")
        return RootStart(
            tab = if (meeting != null) RootTab.Meetings else if (topic != null) RootTab.Topics else tab ?: RootTab.Today,
            meetingId = meeting,
            topicKey = topic,
            // `whimbrel_demo_devices` opens Settings on the Devices screen with devices waiting; `whimbrel_demo_prep` opens the prep sheet.
            devicesOpen = intent.getBooleanExtra("whimbrel_demo_devices", false),
            prepOpen = intent.getBooleanExtra("whimbrel_demo_prep", false),
            settingsOpen = intent.getBooleanExtra("whimbrel_demo_settings", false) || intent.getBooleanExtra("whimbrel_demo_devices", false),
        )
    }

    /** The component gallery: `--ez whimbrel_gallery true`. It reads no account and no store. */
    fun gallery(intent: Intent): Boolean = intent.getBooleanExtra("whimbrel_gallery", false)

    @Composable
    fun Gallery(intent: Intent) = WGallery(intent.getBooleanExtra("whimbrel_gallery_hc", false), intent.getStringExtra("whimbrel_gallery_section"))
}
