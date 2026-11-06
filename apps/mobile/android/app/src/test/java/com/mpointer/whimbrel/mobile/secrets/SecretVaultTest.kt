package com.mpointer.whimbrel.mobile.secrets

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Test

class SecretVaultTest {
    private val dir = Files.createTempDirectory("vault").toFile().resolve("secrets")
    private val vault = SecretVault(dir, FakeCipher())

    @Test
    fun putThenGetRoundTrips() {
        vault.put("group-key", "s3cret-value")
        assertEquals("s3cret-value", vault.get("group-key"))
    }

    @Test
    fun getOfMissingNameIsNull() = assertNull(vault.get("nope"))

    @Test
    fun deleteRemoves() {
        vault.put("a", "1")
        vault.delete("a")
        assertNull(vault.get("a"))
        vault.delete("a") // deleting a missing name is fine
    }

    @Test
    fun corruptFileReadsAsNullNotAThrow() {
        vault.put("a", "1")
        dir.resolve("a").writeBytes(byteArrayOf(1, 2, 3))
        assertNull(vault.get("a"))
        dir.resolve("a").writeBytes(byteArrayOf())
        assertNull(vault.get("a"))
    }

    @Test
    fun fileOnDiskDoesNotContainThePlaintext() {
        vault.put("a", "plaintext-marker-value")
        assertFalse(String(dir.resolve("a").readBytes(), Charsets.ISO_8859_1).contains("plaintext-marker-value"))
    }

    @Test
    fun overwriteReplaces() {
        vault.put("a", "old")
        vault.put("a", "new")
        assertEquals("new", vault.get("a"))
        assertEquals(listOf("a"), dir.list()!!.toList()) // no temp file left behind
    }

    @Test
    fun valueFromAnotherKeyReadsAsNull() {
        vault.put("a", "1")
        assertNull(SecretVault(dir, FakeCipher(key = 0x11)).get("a"))
    }

    @Test(expected = IllegalArgumentException::class)
    fun namesCannotEscapeTheDirectory() = vault.put("../evil", "x")
}
