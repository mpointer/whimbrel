package com.mpointer.whimbrel.mobile.core

internal object Native : CoreNative {
    init {
        System.loadLibrary("whimbrel_jni")
    }

    override fun coreVersion(): String = nCoreVersion().fromCOutput() ?: ""
    override fun memoryOpen(path: String): Long = nMemoryOpen(path.toCString())
    override fun memoryClose(handle: Long) = nMemoryClose(handle)
    override fun memorySetDeviceId(handle: Long, deviceId: String): Boolean = nMemorySetDeviceId(handle, deviceId.toCString())
    override fun memoryInsertMeeting(handle: Long, meetingJson: String): String? =
        nMemoryInsertMeeting(handle, meetingJson.toCString()).fromCOutput()
    override fun memoryGetMeeting(handle: Long, id: Long): String? = nMemoryGetMeeting(handle, id).fromCOutput()

    override fun syncGenerateDeviceKeypair(): String? = nSyncGenerateDeviceKeypair().fromCOutput()
    override fun syncDevicePublicKey(secretKeyBase64: String): String? =
        nSyncDevicePublicKey(secretKeyBase64.toCString()).fromCOutput()
    override fun deviceComparisonCode(publicKeyBase64: String): String? =
        nDeviceComparisonCode(publicKeyBase64.toCString()).fromCOutput()
    override fun syncUnwrapGroupKey(recipientSecretKeyBase64: String, senderPublicKeyBase64: String, wrappedGroupKeyBase64: String): String? =
        nSyncUnwrapGroupKey(recipientSecretKeyBase64.toCString(), senderPublicKeyBase64.toCString(), wrappedGroupKeyBase64.toCString()).fromCOutput()
    override fun syncGroupKeyDecision(wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApprovedDeviceExists: Boolean): String? =
        nSyncGroupKeyDecision(wrappedKeyPresent, localKeyHeld, anotherApprovedDeviceExists).fromCOutput()
    override fun syncWorkerAuthenticateGoogle(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?): String? =
        nSyncWorkerAuthenticateGoogle(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString(), (platform ?: "").toCString()).fromCOutput()
    override fun syncWorkerListDevices(baseUrl: String, idToken: String, devicePublicKey: String): String? =
        nSyncWorkerListDevices(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString()).fromCOutput()
    override fun syncWorkerRevokeDevice(baseUrl: String, idToken: String, devicePublicKey: String, targetDeviceId: String): String? =
        nSyncWorkerRevokeDevice(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString(), targetDeviceId.toCString()).fromCOutput()
    override fun syncWorkerSetDeviceName(baseUrl: String, idToken: String, devicePublicKey: String, name: String): String? =
        nSyncWorkerSetDeviceName(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString(), name.toCString()).fromCOutput()
    override fun syncWorkerSwapDevice(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?, replaceDeviceId: String): String? =
        nSyncWorkerSwapDevice(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString(), (platform ?: "").toCString(), replaceDeviceId.toCString()).fromCOutput()
    override fun syncWorkerJoinWaitlist(baseUrl: String, idToken: String, devicePublicKey: String, wantsJson: String, platform: String?): String? =
        nSyncWorkerJoinWaitlist(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString(), wantsJson.toCString(), (platform ?: "").toCString()).fromCOutput()
    override fun syncWorkerAccountPlan(baseUrl: String, idToken: String, devicePublicKey: String): String? =
        nSyncWorkerAccountPlan(baseUrl.toCString(), idToken.toCString(), devicePublicKey.toCString()).fromCOutput()
    override fun syncWorkerErrorIsPermanent(errorJson: String): Boolean = nSyncWorkerErrorIsPermanent(errorJson.toCString())
    override fun syncWorkerErrorCode(errorJson: String): String? = nSyncWorkerErrorCode(errorJson.toCString()).fromCOutput()
    override fun oauthJwtClaims(jwt: String): String? = nOauthJwtClaims(jwt.toCString()).fromCOutput()

    override fun syncContentPullOnly(handle: Long, tursoUrl: String, tursoToken: String, groupKeyBase64: String, deviceId: String, captureWindowDays: Int): Boolean =
        nSyncContentPullOnly(handle, tursoUrl.toCString(), tursoToken.toCString(), groupKeyBase64.toCString(), deviceId.toCString(), captureWindowDays)
    override fun syncContentLastError(): String? = nSyncContentLastError().fromCOutput()
    override fun syncContentMoreToPull(): Boolean = nSyncContentMoreToPull()
    override fun syncContentLastPullStats(): String? = nSyncContentLastPullStats().fromCOutput()
    override fun syncContentOlderHistoryLoading(handle: Long): Boolean = nSyncContentOlderHistoryLoading(handle)
    override fun meetingsGroupedCount(handle: Long): Long = nMeetingsGroupedCount(handle)
    override fun meetingsRecentGrouped(handle: Long, limit: Long): String? = nMeetingsRecentGrouped(handle, limit).fromCOutput()
    override fun syncSettingsIteration(tursoUrl: String, tursoToken: String, groupKeyBase64: String, localJson: String): String? =
        nSyncSettingsIteration(tursoUrl.toCString(), tursoToken.toCString(), groupKeyBase64.toCString(), localJson.toCString()).fromCOutput()
    override fun syncSettingsApplyPlan(pulledJson: String, localTimestampsJson: String, apiKeySyncEnabledJson: String): String? =
        nSyncSettingsApplyPlan(pulledJson.toCString(), localTimestampsJson.toCString(), apiKeySyncEnabledJson.toCString()).fromCOutput()
    override fun citationsResolve(handle: Long, refsJson: String): String? = nCitationsResolve(handle, refsJson.toCString()).fromCOutput()
    override fun memoryGetCaptureItem(handle: Long, id: Long): String? = nMemoryGetCaptureItem(handle, id).fromCOutput()

    override fun memoryGetTranscriptSegmentsPage(handle: Long, meetingId: Long, offset: Long, limit: Long): String? =
        nMemoryGetTranscriptSegmentsPage(handle, meetingId, offset, limit).fromCOutput()
    override fun memoryListMeetingActionItems(handle: Long, meetingId: Long): String? = nMemoryListMeetingActionItems(handle, meetingId).fromCOutput()
    override fun routinesRecentRuns(handle: Long, limit: Long): String? = nRoutinesRecentRuns(handle, limit).fromCOutput()
    override fun meetingPrepGet(handle: Long, eventId: String): String? = nMeetingPrepGet(handle, eventId.toCString()).fromCOutput()
    override fun memoryUpcomingCalendarEvents(handle: Long, fromUnixMs: Long, untilUnixMs: Long, limit: Long): String? =
        nMemoryUpcomingCalendarEvents(handle, fromUnixMs, untilUnixMs, limit).fromCOutput()
    override fun topicSnapshotsJson(handle: Long): String? = nTopicSnapshotsJson(handle).fromCOutput()

    private external fun nCoreVersion(): ByteArray?
    private external fun nMemoryOpen(path: ByteArray): Long
    private external fun nMemoryClose(handle: Long)
    private external fun nMemorySetDeviceId(handle: Long, deviceId: ByteArray): Boolean
    private external fun nMemoryInsertMeeting(handle: Long, meetingJson: ByteArray): ByteArray?
    private external fun nMemoryGetMeeting(handle: Long, id: Long): ByteArray?
    private external fun nSyncGenerateDeviceKeypair(): ByteArray?
    private external fun nSyncDevicePublicKey(secretKey: ByteArray): ByteArray?
    private external fun nDeviceComparisonCode(publicKey: ByteArray): ByteArray?
    private external fun nSyncUnwrapGroupKey(recipientSecret: ByteArray, senderPublic: ByteArray, wrapped: ByteArray): ByteArray?
    private external fun nSyncGroupKeyDecision(wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApproved: Boolean): ByteArray?
    private external fun nSyncWorkerAuthenticateGoogle(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray, platform: ByteArray): ByteArray?
    private external fun nSyncWorkerListDevices(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray): ByteArray?
    private external fun nSyncWorkerRevokeDevice(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray, targetDeviceId: ByteArray): ByteArray?
    private external fun nSyncWorkerSetDeviceName(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray, name: ByteArray): ByteArray?
    private external fun nSyncWorkerSwapDevice(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray, platform: ByteArray, replaceDeviceId: ByteArray): ByteArray?
    private external fun nSyncWorkerJoinWaitlist(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray, wantsJson: ByteArray, platform: ByteArray): ByteArray?
    private external fun nSyncWorkerAccountPlan(baseUrl: ByteArray, idToken: ByteArray, devicePublicKey: ByteArray): ByteArray?
    private external fun nSyncWorkerErrorIsPermanent(errorJson: ByteArray): Boolean
    private external fun nSyncWorkerErrorCode(errorJson: ByteArray): ByteArray?
    private external fun nOauthJwtClaims(jwt: ByteArray): ByteArray?
    private external fun nSyncContentPullOnly(handle: Long, url: ByteArray, token: ByteArray, groupKey: ByteArray, deviceId: ByteArray, captureWindowDays: Int): Boolean
    private external fun nSyncContentLastError(): ByteArray?
    private external fun nSyncContentMoreToPull(): Boolean
    private external fun nSyncContentLastPullStats(): ByteArray?
    private external fun nSyncContentOlderHistoryLoading(handle: Long): Boolean
    private external fun nMeetingsGroupedCount(handle: Long): Long
    private external fun nMeetingsRecentGrouped(handle: Long, limit: Long): ByteArray?
    private external fun nSyncSettingsIteration(url: ByteArray, token: ByteArray, groupKey: ByteArray, localJson: ByteArray): ByteArray?
    private external fun nSyncSettingsApplyPlan(pulled: ByteArray, localTimestamps: ByteArray, apiKeySyncEnabled: ByteArray): ByteArray?
    private external fun nCitationsResolve(handle: Long, refsJson: ByteArray): ByteArray?
    private external fun nMemoryGetCaptureItem(handle: Long, id: Long): ByteArray?
    private external fun nMemoryGetTranscriptSegmentsPage(handle: Long, meetingId: Long, offset: Long, limit: Long): ByteArray?
    private external fun nMemoryListMeetingActionItems(handle: Long, meetingId: Long): ByteArray?
    private external fun nRoutinesRecentRuns(handle: Long, limit: Long): ByteArray?
    private external fun nMeetingPrepGet(handle: Long, eventId: ByteArray): ByteArray?
    private external fun nMemoryUpcomingCalendarEvents(handle: Long, from: Long, until: Long, limit: Long): ByteArray?
    private external fun nTopicSnapshotsJson(handle: Long): ByteArray?
}
