package com.mpointer.whimbrel.mobile.theme

/** The shared theme files, on the test classpath via build.gradle.kts. */
object ThemeTestFiles {
    fun read(path: String): String =
        checkNotNull(javaClass.classLoader!!.getResourceAsStream(path)) { "missing test resource $path" }.use { String(it.readBytes()) }

    fun invalid(name: String) = read("invalid/$name.json")
}
