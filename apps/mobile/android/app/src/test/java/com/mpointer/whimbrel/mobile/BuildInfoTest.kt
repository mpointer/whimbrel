package com.mpointer.whimbrel.mobile

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Test

class BuildInfoTest {
    // Gradle runs unit tests from apps/mobile/android/app.
    private val repoVersion = File("../../../../VERSION").readText().trim()

    @Test
    fun versionNameIsTheRepoVersionFile() {
        assertEquals(repoVersion, BuildConfig.VERSION_NAME)
    }

    @Test
    fun versionCodeIsMajorMinorPatch() {
        val (major, minor, patch) = repoVersion.split(".").map { it.toInt() }
        assertEquals(major * 1_000_000 + minor * 1_000 + patch, BuildConfig.VERSION_CODE)
    }
}
