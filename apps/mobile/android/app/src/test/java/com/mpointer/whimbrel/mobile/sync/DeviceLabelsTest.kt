package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import org.junit.Assert.assertEquals
import org.junit.Test

class DeviceLabelsTest {
    private fun record(publicKey: String, name: String?) =
        SyncDeviceRecord(id = "id-$publicKey", accountId = "a", publicKey = publicKey, name = name, status = "approved", enrolledAtUnixMs = 0)

    private fun DeviceLabels.text(id: String) = label(id).localized(StringCatalog.resolve)

    @Test
    fun `namesComeFromTheDeviceListKeyedByPublicKey`() {
        val names = DeviceLabels.names(
            listOf(record("pk-1", "  Mac  "), record("pk-1", "Second"), record("pk-2", "   "), record("pk-3", null), record("pk-4", "Pixel")),
        )
        assertEquals(mapOf("pk-1" to "Mac", "pk-4" to "Pixel"), names)
    }

    @Test
    fun `labelPrefersTheNameThenThisPhoneThenAShortId`() {
        val labels = DeviceLabels(names = mapOf("pk-own" to "My Pixel", "pk-mac" to "Mac"), ownDeviceId = "pk-own")
        assertEquals("Mac", labels.text("pk-mac"))
        assertEquals("My Pixel", labels.text("pk-own"))
        val unnamed = DeviceLabels(ownDeviceId = "pk-own")
        assertEquals("This phone", unnamed.text("pk-own"))
        assertEquals("Device abcdef", unnamed.text("abcdefghij"))
    }
}
