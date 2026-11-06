package com.mpointer.whimbrel.mobile

import android.content.Intent
import com.mpointer.whimbrel.mobile.ui.RootStart

/** Release builds have no demo mode: the demo extras are ignored. */
@Suppress("UNUSED_PARAMETER")
object DemoHooks {
    fun apply(app: WhimbrelApplication, intent: Intent) {}

    fun start(intent: Intent) = RootStart()
}
