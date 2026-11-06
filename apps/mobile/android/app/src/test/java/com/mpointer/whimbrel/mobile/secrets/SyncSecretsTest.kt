package com.mpointer.whimbrel.mobile.secrets

import java.nio.file.Files
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class SyncSecretsTest {
    private val dir = Files.createTempDirectory("vault").toFile()
    private val secrets = SyncSecrets(SecretVault(dir, FakeCipher()))

    @Test
    fun groupKeyRoundTripsAndPersistsAcrossInstances() {
        assertNull(secrets.groupKey)
        secrets.groupKey = "Z3JvdXA="
        assertEquals("Z3JvdXA=", secrets.groupKey)
        assertEquals("Z3JvdXA=", SyncSecrets(SecretVault(dir, FakeCipher())).groupKey)
    }

    @Test
    fun deletesAreIndependent() {
        secrets.deviceSecretKey = "dev"
        secrets.groupKey = "grp"
        secrets.deleteGroupKey()
        assertNull(secrets.groupKey)
        assertEquals("dev", secrets.deviceSecretKey)
        secrets.groupKey = "grp"
        secrets.deleteDeviceSecretKey()
        assertNull(secrets.deviceSecretKey)
        assertEquals("grp", secrets.groupKey)
    }

    @Test
    fun usesTheSameNamesAsTheAppleKeychains() {
        secrets.deviceSecretKey = "d"
        secrets.groupKey = "g"
        assertEquals(setOf("device-secret-key", "group-key"), dir.list()!!.toSet())
    }
}
