package com.mpointer.whimbrel.mobile.core

import java.io.File
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

private class FakeNative(private val handle: Long) : FakeSyncNative() {
    var closes = 0
    override fun memoryOpen(path: String) = handle
    override fun memoryClose(handle: Long) { closes++ }
}

class CoreStoreTest {
    @Test
    fun openReturnsNullWhenCoreFails() = assertNull(CoreStore.open(File("x"), FakeNative(0)))

    @Test
    fun closeTwiceClosesOnce() {
        val fake = FakeNative(7)
        val store = CoreStore.open(File("x"), fake)!!
        store.close()
        store.close()
        assertEquals(1, fake.closes)
    }
}
