package com.mpointer.whimbrel.mobile.core

/** The String-typed seam over the Rust core; models take this so tests can fake it. */
interface CoreNative {
    fun coreVersion(): String

    /** Opens the store at [path] with no models dir; 0 on failure. */
    fun memoryOpen(path: String): Long
    fun memoryClose(handle: Long)
    fun memorySetDeviceId(handle: Long, deviceId: String): Boolean
    fun memoryInsertMeeting(handle: Long, meetingJson: String): String?
    fun memoryGetMeeting(handle: Long, id: Long): String?

    // Device enrollment. Each returns the core's JSON (or plain text) verbatim, null when the core returns NULL.
    fun syncGenerateDeviceKeypair(): String?
    fun syncDevicePublicKey(secretKeyBase64: String): String?
    fun deviceComparisonCode(publicKeyBase64: String): String?
    fun syncUnwrapGroupKey(recipientSecretKeyBase64: String, senderPublicKeyBase64: String, wrappedGroupKeyBase64: String): String?
    fun syncGroupKeyDecision(wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApprovedDeviceExists: Boolean): String?

    /** Worker calls block for up to 30 s: never on the main thread. A null [platform] sends none. */
    fun syncWorkerAuthenticateGoogle(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?): String?
    fun syncWorkerListDevices(baseUrl: String, idToken: String, devicePublicKey: String): String?
    fun syncWorkerRevokeDevice(baseUrl: String, idToken: String, devicePublicKey: String, targetDeviceId: String): String?
    fun syncWorkerSetDeviceName(baseUrl: String, idToken: String, devicePublicKey: String, name: String): String?
    fun syncWorkerSwapDevice(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?, replaceDeviceId: String): String?
    fun syncWorkerJoinWaitlist(baseUrl: String, idToken: String, devicePublicKey: String, wantsJson: String, platform: String?): String?
    fun syncWorkerErrorIsPermanent(errorJson: String): Boolean
    fun syncWorkerErrorCode(errorJson: String): String?
    fun oauthJwtClaims(jwt: String): String?

    // Content and settings sync. The sync calls block (up to 30 s): never on the main thread.
    fun syncContentPullOnly(handle: Long, tursoUrl: String, tursoToken: String, groupKeyBase64: String, deviceId: String, captureWindowDays: Int): Boolean
    fun syncContentLastError(): String?
    fun syncContentMoreToPull(): Boolean
    fun syncContentLastPullStats(): String? = null
    /** Meetings after grouping copies, over every meeting (no cap); -1 on failure. */
    fun meetingsGroupedCount(handle: Long): Long
    fun meetingsRecentGrouped(handle: Long, limit: Long): String?
    fun syncSettingsIteration(tursoUrl: String, tursoToken: String, groupKeyBase64: String, localJson: String): String?
    fun syncSettingsApplyPlan(pulledJson: String, localTimestampsJson: String, apiKeySyncEnabledJson: String): String?
    fun citationsResolve(handle: Long, refsJson: String): String?
    fun memoryGetCaptureItem(handle: Long, id: Long): String?

    // Read side.
    fun memoryGetTranscriptSegmentsPage(handle: Long, meetingId: Long, offset: Long, limit: Long): String?
    fun memoryListMeetingActionItems(handle: Long, meetingId: Long): String?
    fun routinesRecentRuns(handle: Long, limit: Long): String?
    fun meetingPrepGet(handle: Long, eventId: String): String?
    fun memoryUpcomingCalendarEvents(handle: Long, fromUnixMs: Long, untilUnixMs: Long, limit: Long): String?
}
