package com.mpointer.whimbrel.mobile.core

import java.io.File
import org.junit.Assert.assertTrue
import org.junit.Test

/** A rename or an R8 strip must not silently break JNI linking. Gradle runs JVM tests from the app/ directory. */
class NativeKeepRulesTest {
    private fun text(path: String) = File(path).readText()

    @Test
    fun everyExternalFunHasAJniSymbol() {
        val names = Regex("""external fun (\w+)""").findAll(text("src/main/java/com/mpointer/whimbrel/mobile/core/Native.kt"))
            .map { it.groupValues[1] }.toList()
        assertTrue("no external funs found", names.isNotEmpty())
        val jni = text("src/main/cpp/whimbrel_jni.c")
        val missing = names.filter { !Regex("""\bJNI\(\s*$it\s*\)|Java_com_mpointer_whimbrel_mobile_core_Native_$it\b""").containsMatchIn(jni) }
        assertTrue("no JNI symbol for $missing", missing.isEmpty())
    }

    @Test
    fun nativeKeepRuleIsPresent() {
        val rules = text("proguard-rules.pro").replace(Regex("\\s+"), " ")
        assertTrue(rules.contains("-keep class com.mpointer.whimbrel.mobile.core.Native { native <methods>; }"))
    }
}
