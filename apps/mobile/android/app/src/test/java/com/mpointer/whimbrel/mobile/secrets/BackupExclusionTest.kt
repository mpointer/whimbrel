package com.mpointer.whimbrel.mobile.secrets

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/** Pins "a restore brings back no secrets": backup is off, and both rule files exclude every domain anyway. */
class BackupExclusionTest {
    private fun src(path: String) = File("src/main/$path").readText()
    private val domains = listOf("root", "file", "sharedpref", "database", "external")

    @Test
    fun manifestDisablesBackup() {
        val manifest = src("AndroidManifest.xml")
        assertTrue(manifest.contains("""android:allowBackup="false""""))
        assertTrue(manifest.contains("""android:dataExtractionRules="@xml/data_extraction_rules""""))
        assertTrue(manifest.contains("""android:fullBackupContent="@xml/backup_rules""""))
    }

    @Test
    fun backupXmlsExcludeEveryDomain() {
        val rules = src("res/xml/data_extraction_rules.xml")
        for (section in listOf("cloud-backup", "device-transfer")) {
            val body = rules.substringAfter("<$section>").substringBefore("</$section>")
            for (d in domains) assertTrue("$section excludes $d", body.contains("""<exclude domain="$d" />"""))
        }
        val legacy = src("res/xml/backup_rules.xml")
        for (d in domains) assertTrue("backup_rules excludes $d", legacy.contains("""<exclude domain="$d" />"""))
    }
}
