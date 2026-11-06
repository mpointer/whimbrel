package com.mpointer.whimbrel.mobile.sync

import java.io.File

object StorePaths {
    /** `noBackupFilesDir/Sync/memory.sqlite`, creating `Sync/`. */
    fun storeFile(noBackupFilesDir: File): File {
        val dir = File(noBackupFilesDir, "Sync").apply { mkdirs() }
        return File(dir, "memory.sqlite")
    }
}
