package com.mpointer.whimbrel.mobile.core

object CpuFeatures {
    /** True when a `Features` line of /proc/cpuinfo lists `fphp` or `asimdhp` (ARMv8.2 FP16). */
    fun hasFp16(cpuinfo: String): Boolean = cpuinfo.lineSequence()
        .filter { it.trimStart().startsWith("Features") }
        .any { line -> line.substringAfter(':').split(' ', '\t').any { it == "fphp" || it == "asimdhp" } }
}
