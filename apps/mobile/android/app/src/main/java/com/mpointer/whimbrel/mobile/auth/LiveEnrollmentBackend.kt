package com.mpointer.whimbrel.mobile.auth

import android.app.Activity
import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import com.mpointer.whimbrel.mobile.core.SyncWorkerClient
import com.mpointer.whimbrel.mobile.sync.EnrollmentBackend

/** The one concrete backend. [activity] is the started MainActivity (null in the background). */
class LiveEnrollmentBackend(
    private val google: GoogleSignIn,
    private val worker: SyncWorkerClient,
    private val activity: () -> Activity?,
) : EnrollmentBackend {
    override suspend fun signIn() = google.authorize(activity() ?: throw GoogleSignInError.Failed("no activity"))
    override suspend fun idToken(): String = google.validIdToken()
    override fun signOut() = google.signOut()

    override suspend fun authenticate(idToken: String, devicePublicKey: String): GoogleAuthResult =
        worker.authenticate(idToken, devicePublicKey)

    override suspend fun listDevices(idToken: String, devicePublicKey: String): List<SyncDeviceRecord> =
        worker.listDevices(idToken, devicePublicKey)

    override suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) =
        worker.revoke(idToken, devicePublicKey, targetDeviceId)

    override suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) =
        worker.setDeviceName(idToken, devicePublicKey, name)

    override suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) =
        worker.swap(idToken, devicePublicKey, replaceDeviceId)

    override suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) =
        worker.joinWaitlist(idToken, devicePublicKey, wants)

    override suspend fun accountPlan(idToken: String, devicePublicKey: String): AccountPlan =
        worker.accountPlan(idToken, devicePublicKey)
}
