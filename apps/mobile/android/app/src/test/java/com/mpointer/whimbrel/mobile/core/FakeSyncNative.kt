package com.mpointer.whimbrel.mobile.core

/** A [CoreNative] whose Worker calls all return [reply]; every call's first JSON-bearing argument is recorded. */
internal open class FakeSyncNative(var reply: String? = null) : CoreNative {
    var permanentArg: String? = null
    var errorCodeArg: String? = null
    var errorCodeReply: String? = null
    var permanentReply = false
    var lastPlatform: String? = "unset" // sentinel: a recorded null means the call passed no platform
    var lastWants: String? = null

    override fun coreVersion() = "0"
    override fun memoryOpen(path: String) = 0L
    override fun memoryClose(handle: Long) {}
    override fun memorySetDeviceId(handle: Long, deviceId: String): Boolean {
        calls += "setDeviceId"
        return true
    }
    override fun memoryInsertMeeting(handle: Long, meetingJson: String): String? = null
    override fun memoryGetMeeting(handle: Long, id: Long): String? = memoryGetMeetingReply

    override fun syncGenerateDeviceKeypair(): String? = null
    override fun syncDevicePublicKey(secretKeyBase64: String): String? = null
    override fun deviceComparisonCode(publicKeyBase64: String): String? = null
    override fun syncUnwrapGroupKey(recipientSecretKeyBase64: String, senderPublicKeyBase64: String, wrappedGroupKeyBase64: String): String? = null
    override fun syncGroupKeyDecision(wrappedKeyPresent: Boolean, localKeyHeld: Boolean, anotherApprovedDeviceExists: Boolean): String? = reply
    override fun syncWorkerAuthenticateGoogle(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?): String? {
        lastPlatform = platform
        return reply
    }
    override fun syncWorkerListDevices(baseUrl: String, idToken: String, devicePublicKey: String): String? = reply
    override fun syncWorkerRevokeDevice(baseUrl: String, idToken: String, devicePublicKey: String, targetDeviceId: String): String? = reply
    override fun syncWorkerSetDeviceName(baseUrl: String, idToken: String, devicePublicKey: String, name: String): String? = reply
    override fun syncWorkerSwapDevice(baseUrl: String, idToken: String, devicePublicKey: String, platform: String?, replaceDeviceId: String): String? {
        lastPlatform = platform
        return reply
    }
    override fun syncWorkerJoinWaitlist(baseUrl: String, idToken: String, devicePublicKey: String, wantsJson: String, platform: String?): String? {
        lastWants = wantsJson
        lastPlatform = platform
        return reply
    }
    override fun syncWorkerErrorIsPermanent(errorJson: String): Boolean {
        permanentArg = errorJson
        return permanentReply
    }
    override fun syncWorkerErrorCode(errorJson: String): String? {
        errorCodeArg = errorJson
        return errorCodeReply
    }
    override fun oauthJwtClaims(jwt: String): String? = null

    val calls = mutableListOf<String>()
    var pullOnlyReply = true
    var pullOnlyArgs: List<Any>? = null
    var lastError: String? = null
    var groupedReply: String? = null
    var iterationReply: String? = null
    var planReply: String? = null
    var planArgs: List<String>? = null

    override fun syncContentPullOnly(handle: Long, tursoUrl: String, tursoToken: String, groupKeyBase64: String, deviceId: String, captureWindowDays: Int): Boolean {
        calls += "pull"
        pullOnlyArgs = listOf(handle, tursoUrl, tursoToken, groupKeyBase64, deviceId, captureWindowDays)
        return pullOnlyReply
    }
    override fun syncContentLastError(): String? = lastError
    var moreToPullReply = false
    override fun syncContentMoreToPull(): Boolean = moreToPullReply
    var groupedCountReply = -1L
    override fun meetingsGroupedCount(handle: Long): Long = groupedCountReply
    override fun meetingsRecentGrouped(handle: Long, limit: Long): String? = groupedReply
    override fun syncSettingsIteration(tursoUrl: String, tursoToken: String, groupKeyBase64: String, localJson: String): String? = iterationReply
    override fun syncSettingsApplyPlan(pulledJson: String, localTimestampsJson: String, apiKeySyncEnabledJson: String): String? {
        planArgs = listOf(pulledJson, localTimestampsJson, apiKeySyncEnabledJson)
        return planReply
    }
    override fun citationsResolve(handle: Long, refsJson: String): String? = null
    override fun memoryGetCaptureItem(handle: Long, id: Long): String? = null

    var memoryGetMeetingReply: String? = null
    var transcriptReply: String? = null
    var actionItemsReply: String? = null
    var runsReply: String? = null
    var prepReply: String? = null
    var upcomingReply: String? = null
    override fun memoryGetTranscriptSegmentsPage(handle: Long, meetingId: Long, offset: Long, limit: Long): String? = transcriptReply
    override fun memoryListMeetingActionItems(handle: Long, meetingId: Long): String? = actionItemsReply
    override fun routinesRecentRuns(handle: Long, limit: Long): String? = runsReply
    override fun meetingPrepGet(handle: Long, eventId: String): String? = prepReply
    override fun memoryUpcomingCalendarEvents(handle: Long, fromUnixMs: Long, untilUnixMs: Long, limit: Long): String? = upcomingReply
}
