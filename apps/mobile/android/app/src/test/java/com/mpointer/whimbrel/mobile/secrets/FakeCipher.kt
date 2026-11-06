package com.mpointer.whimbrel.mobile.secrets

/** Reversible and detectably tampered: XOR with a key byte, last byte is a checksum of the rest. */
class FakeCipher(private val key: Int = 0x5A) : SecretCipher {
    override fun encrypt(plain: ByteArray): ByteArray {
        val body = ByteArray(plain.size) { (plain[it].toInt() xor key).toByte() }
        return body + body.fold(key) { a, b -> a xor b.toInt() }.toByte()
    }

    override fun decrypt(blob: ByteArray): ByteArray {
        require(blob.isNotEmpty()) { "empty" }
        val body = blob.copyOf(blob.size - 1)
        require(body.fold(key) { a, b -> a xor b.toInt() }.toByte() == blob.last()) { "tampered" }
        return ByteArray(body.size) { (body[it].toInt() xor key).toByte() }
    }
}
