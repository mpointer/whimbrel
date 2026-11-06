package com.mpointer.whimbrel.mobile

import android.content.Intent
import com.mpointer.whimbrel.mobile.read.DemoSyncStore
import com.mpointer.whimbrel.mobile.read.DemoSyncedBackend
import com.mpointer.whimbrel.mobile.ui.RootStart
import com.mpointer.whimbrel.mobile.ui.RootTab

/**
 * Debug launch extras for screenshots: `adb shell am start --ez whimbrel_demo_synced true --es whimbrel_demo_tab meetings
 * --el whimbrel_demo_meeting 3 --ez whimbrel_demo_settings true`. Nothing here touches real state.
 */
object DemoHooks {
    fun apply(app: WhimbrelApplication, intent: Intent) {
        if (intent.getBooleanExtra("whimbrel_demo_synced", false)) app.startDemo(DemoSyncStore(), DemoSyncedBackend())
    }

    /** The shell's first tab, meeting and sheet; a meeting implies the Meetings tab, as on the iPhone. */
    fun start(intent: Intent): RootStart {
        val meeting = intent.getLongExtra("whimbrel_demo_meeting", -1L).takeIf { it >= 0 }
        val tab = intent.getStringExtra("whimbrel_demo_tab")?.let { name -> RootTab.entries.firstOrNull { it.name.equals(name, ignoreCase = true) } }
        return RootStart(
            tab = if (meeting != null) RootTab.Meetings else tab ?: RootTab.Today,
            meetingId = meeting,
            settingsOpen = intent.getBooleanExtra("whimbrel_demo_settings", false),
        )
    }
}
