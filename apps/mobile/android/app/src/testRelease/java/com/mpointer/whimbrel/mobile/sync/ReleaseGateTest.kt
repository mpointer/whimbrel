package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.BuildConfig
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import com.mpointer.whimbrel.mobile.core.SyncWorkerClient
import com.mpointer.whimbrel.mobile.secrets.FakeCipher
import com.mpointer.whimbrel.mobile.secrets.SecretVault
import com.mpointer.whimbrel.mobile.secrets.SyncSecrets
import java.nio.file.Files
import kotlinx.coroutines.ExperimentalCoroutinesApi
import kotlinx.coroutines.test.runTest
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

/**
 * Runs only against the release variant (:app:testReleaseUnitTest, in CI): a release build must not approve
 * devices until Mike's real two-device test (PR #914). Delete with the gate.
 */
@OptIn(ExperimentalCoroutinesApi::class)
class ReleaseGateTest {
    @Test
    fun releaseBuildHasDeviceApprovalOff() {
        assertFalse(BuildConfig.DEVICE_APPROVAL_ENABLED)
        assertFalse(DeviceApproval.enabled)
    }

    @Test
    fun aDefaultModelNeitherWrapsNorSendsInARelease() = runTest {
        val backend = FakeBackend()
        val native = FakeCoreNative()
        val secrets = SyncSecrets(SecretVault(Files.createTempDirectory("vault").toFile(), FakeCipher()))
        secrets.deviceSecretKey = "sk-me"
        secrets.groupKey = "group-key"
        backend.devices = listOf(
            SyncDeviceRecord(id = "new", accountId = "acct", publicKey = "pk-new", name = "Mac", platform = "macos", status = "pending", enrolledAtUnixMs = 0),
        )
        // approvalEnabled deliberately not passed: the default is what ships.
        val model = EnrollmentModel(
            backend = backend, secrets = secrets, native = native,
            isPermanentError = SyncWorkerClient(native, "")::isPermanent,
            scope = backgroundScope, deviceName = { "Pixel" }, clock = { 0L }, sleep = { },
        )
        model.refreshOtherDevices()
        assertEquals(1, model.otherDevices.value.size)
        model.approve("new", "pk-new")
        assertTrue("no key wrapped", native.wraps.isEmpty())
        assertTrue("nothing sent", backend.approved.isEmpty())
    }
}
