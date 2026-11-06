package com.mpointer.whimbrel.mobile.secrets

/** The sync key material, under the names `SyncKeychain` uses on Apple platforms. Setting null deletes. */
class SyncSecrets(private val vault: SecretVault) {
    var deviceSecretKey: String?
        get() = vault.get(DEVICE_SECRET_KEY)
        set(value) = store(DEVICE_SECRET_KEY, value)

    var groupKey: String?
        get() = vault.get(GROUP_KEY)
        set(value) = store(GROUP_KEY, value)

    fun deleteDeviceSecretKey() = vault.delete(DEVICE_SECRET_KEY)

    /**
     * The Worker's Turso token with its database URL and when it was issued, so a process restart reuses
     * it instead of asking Google again (which, with two Google accounts, shows a chooser). Kept with the
     * other keys: device-only, never backed up, deleted by a wipe.
     */
    var tursoSession: TursoSession?
        get() = vault.get(TURSO_SESSION)?.let(TursoSession::decode)
        set(value) = store(TURSO_SESSION, value?.encode())

    fun deleteGroupKey() = vault.delete(GROUP_KEY)

    private fun store(name: String, value: String?) = if (value == null) vault.delete(name) else vault.put(name, value)

    private companion object {
        const val DEVICE_SECRET_KEY = "device-secret-key"
        const val GROUP_KEY = "group-key"
        const val TURSO_SESSION = "turso-session"
    }
}

data class TursoSession(val token: String, val databaseUrl: String, val issuedAtMs: Long) {
    fun encode(): String = "$issuedAtMs\n$databaseUrl\n$token"

    companion object {
        fun decode(s: String): TursoSession? {
            val parts = s.split("\n", limit = 3)
            if (parts.size != 3) return null
            val at = parts[0].toLongOrNull() ?: return null
            return TursoSession(parts[2], parts[1], at)
        }
    }
}
