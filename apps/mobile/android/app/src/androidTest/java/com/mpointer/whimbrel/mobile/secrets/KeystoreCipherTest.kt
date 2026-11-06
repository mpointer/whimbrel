package com.mpointer.whimbrel.mobile.secrets

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertFalse
import org.junit.Test

class KeystoreCipherTest {
    private val cipher = KeystoreCipher("whimbrel-secrets-test")

    @Test
    fun roundTrips() {
        val plain = "group-key-bytes".toByteArray()
        val blob = cipher.encrypt(plain)
        assertFalse(blob.contentEquals(plain))
        assertArrayEquals(plain, cipher.decrypt(blob))
    }

    @Test
    fun flippedByteThrows() {
        val blob = cipher.encrypt("x".repeat(40).toByteArray())
        for (i in listOf(0, 12, blob.size - 1)) { // IV, ciphertext, tag
            val bad = blob.copyOf().also { it[i] = (it[i].toInt() xor 1).toByte() }
            try {
                cipher.decrypt(bad)
                throw AssertionError("byte $i flipped but decrypt succeeded")
            } catch (_: java.security.GeneralSecurityException) {
            }
        }
    }
}
