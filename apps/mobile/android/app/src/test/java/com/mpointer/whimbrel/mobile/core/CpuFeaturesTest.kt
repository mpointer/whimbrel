package com.mpointer.whimbrel.mobile.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CpuFeaturesTest {
    @Test
    fun pixelLikeFeaturesHaveFp16() {
        assertTrue(CpuFeatures.hasFp16("processor\t: 0\nFeatures\t: fp asimd evtstrm aes pmull fphp asimdhp cpuid\n"))
    }

    @Test
    fun oldCoreWithoutFp16IsRejected() = assertFalse(CpuFeatures.hasFp16("Features : fp asimd evtstrm"))

    @Test
    fun emptyIsRejected() = assertFalse(CpuFeatures.hasFp16(""))
}
