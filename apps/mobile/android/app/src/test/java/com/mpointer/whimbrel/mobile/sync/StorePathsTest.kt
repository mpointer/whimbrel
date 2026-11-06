package com.mpointer.whimbrel.mobile.sync

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class StorePathsTest {
    @Test
    fun defaultStoreLivesInTheSyncDirectoryUnderNoBackup() {
        val root = Files.createTempDirectory("nobackup").toFile()
        val file = StorePaths.storeFile(root)
        assertEquals("memory.sqlite", file.name)
        assertEquals(root.resolve("Sync"), file.parentFile)
        assertTrue(file.parentFile!!.isDirectory)
    }
}
