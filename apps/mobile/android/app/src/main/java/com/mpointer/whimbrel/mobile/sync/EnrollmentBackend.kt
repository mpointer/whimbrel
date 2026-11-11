package com.mpointer.whimbrel.mobile.sync

import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord

/** The network/Google seam, faked in tests; from `EnrollmentModel.swift`, minus `approve` (the phone never approves). */
interface EnrollmentBackend {
    /** Interactive Google sign-in (consent screen). */
    suspend fun signIn()

    /** A currently valid Google ID token, refreshed silently; throws when not signed in. */
    suspend fun idToken(): String

    suspend fun authenticate(idToken: String, devicePublicKey: String): GoogleAuthResult
    suspend fun listDevices(idToken: String, devicePublicKey: String): List<SyncDeviceRecord>

    /** Revokes a device row; the phone revokes its own when it is approved but cannot get a key. */
    suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String)

    /** Names this phone's own device row (approved devices only). */
    suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String)

    /** "Use this phone instead": this phone becomes a pending swap for [replaceDeviceId]. */
    suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String)

    suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>)

    suspend fun accountPlan(idToken: String, devicePublicKey: String): AccountPlan

    /** Forgets the Google sign-in. */
    fun signOut()
}
