package com.mpointer.whimbrel.mobile.secrets

import java.io.File
import java.nio.file.Files
import java.nio.file.StandardCopyOption

interface SecretCipher {
    fun encrypt(plain: ByteArray): ByteArray

    /** Throws on a wrong key or a tampered blob. */
    fun decrypt(blob: ByteArray): ByteArray
}

/** One encrypted file per name in [dir] (`noBackupFilesDir/secrets` in the app). */
class SecretVault(private val dir: File, private val cipher: SecretCipher) {
    fun put(name: String, value: String) {
        val target = file(name)
        dir.mkdirs()
        val tmp = File(dir, "$name.tmp")
        tmp.writeBytes(cipher.encrypt(value.toByteArray(Charsets.UTF_8)))
        Files.move(tmp.toPath(), target.toPath(), StandardCopyOption.REPLACE_EXISTING, StandardCopyOption.ATOMIC_MOVE)
    }

    /** Null when missing or undecryptable (corrupt file, or a Keystore key lost to a reset). */
    fun get(name: String): String? {
        val f = file(name)
        if (!f.isFile) return null
        return try {
            String(cipher.decrypt(f.readBytes()), Charsets.UTF_8)
        } catch (_: Exception) {
            null
        }
    }

    fun delete(name: String) {
        file(name).delete()
    }

    private fun file(name: String): File {
        require(Regex("[A-Za-z0-9-]+").matches(name)) { "bad secret name" }
        return File(dir, name)
    }
}
