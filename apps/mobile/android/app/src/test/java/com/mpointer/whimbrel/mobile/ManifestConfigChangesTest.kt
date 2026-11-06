package com.mpointer.whimbrel.mobile

import java.io.File
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/** Folding or unfolding must re-lay out MainActivity, not recreate it, so the approval poll and the code survive. */
class ManifestConfigChangesTest {
    @Test
    fun mainActivityHandlesFoldingItself() {
        val manifest = File("src/main/AndroidManifest.xml").readText()
        val handled = Regex("""android:configChanges="([^"]*)"""").find(manifest)!!.groupValues[1].split("|")
        for (change in listOf("orientation", "screenSize", "smallestScreenSize", "screenLayout", "density")) {
            assertTrue("configChanges handles $change", change in handled)
        }
        assertFalse(manifest.contains("screenOrientation"))
    }
}
