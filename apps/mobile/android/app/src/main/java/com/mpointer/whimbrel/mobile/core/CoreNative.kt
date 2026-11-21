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
    fun syncWorkerAccountPlan(baseUrl: String, idToken: String, devicePublicKey: String): String?
    fun syncWorkerErrorIsPermanent(errorJson: String): Boolean
    fun syncWorkerErrorCode(errorJson: String): String?
    fun oauthJwtClaims(jwt: String): String?

    // Content and settings sync. The sync calls block (up to 30 s): never on the main thread.
    fun syncContentPullOnly(handle: Long, tursoUrl: String, tursoToken: String, groupKeyBase64: String, deviceId: String, captureWindowDays: Int): Boolean
    fun syncContentLastError(): String?
    fun syncContentMoreToPull(): Boolean
    fun syncContentLastPullStats(): String? = null
    /** Older history is still being pulled, newest first (#572): read from the store, so it holds across relaunches. */
    fun syncContentOlderHistoryLoading(handle: Long): Boolean = false
    /** `{"pending_devices": n | null}`: other devices that have not announced client deletes yet. */
    fun clientPendingDevicesJson(handle: Long): String? = null
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
    fun topicSnapshotsJson(handle: Long): String?

    // Approving another device.
    /**
     * Wraps this phone's group key to [recipientPublicKeyBase64] with this phone's own secret key:
     * `{"wrapped_group_key_base64": ...}`, or null on malformed input or a low-order recipient key.
     */
    fun syncWrapGroupKey(senderSecretKeyBase64: String, recipientPublicKeyBase64: String, groupKeyBase64: String): String? = null
    /** `POST /v1/devices/:id/approve`: `{"ok": true}` or the error envelope. Blocks: never on the main thread. */
    fun syncWorkerApproveDevice(baseUrl: String, idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String): String? = null

    // Topic links and Ask. The defaults are the "no core" answers, so a test fake only overrides what it uses.
    /** This replica's meeting id for the meeting a topic page names (recording device, its id there); -1 if not here. */
    fun meetingIdForSource(handle: Long, deviceId: String, sourceMeetingId: Long): Long = -1
    /** `{"ok":true,"data":{"prepare_id","prompt"}}` or an error envelope (`error.kind == "spend_cap"`). Store thread. */
    fun llmPrepareAsk(handle: Long, query: String, contextLimit: Long, historyJson: String, routingJson: String): String? = null
    /** `{"citations":[{"kind","id"}]}`; logs usage. Pass -1 for a token count the provider did not report. Store thread. */
    fun llmFinalizeAsk(handle: Long, prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): String? = null
    fun llmDiscardAsk(prepareId: String) {}
    /** `{"run_id":"..."}`; null when the core refuses (bad input, a hosted provider with no key). Never on the store thread. */
    fun inferenceStart(prompt: String, model: String, provider: String, apiKey: String?, baseUrlOverride: String? = null): String? = null
    /** A JSON array of events since the last poll; null for an unknown run. */
    fun inferencePoll(runId: String): String? = null
    fun inferenceCancel(runId: String): Boolean = false
    fun inferenceRelease(runId: String) {}
    /** The model to use: [stored] when it was a real choice, else the core's current default for [provider]. */
    fun modelResolve(provider: String, stored: String?): String? = stored
    fun modelNoteAnswer(handle: Long, provider: String, requested: String, answered: String?, nowUnixMs: Long): Boolean = false
}
