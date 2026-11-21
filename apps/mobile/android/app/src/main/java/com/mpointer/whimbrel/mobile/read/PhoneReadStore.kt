package com.mpointer.whimbrel.mobile.read

/** What the screens read from the phone's replica. Calls block: never on the main thread. */
interface PhoneReadStore {
    /** Grouped meetings, newest first (copies of one call collapse into one entry). */
    fun recentMeetings(limit: Int): List<PhoneMeeting>
    fun meeting(id: Long): PhoneMeeting?
    fun transcriptPage(meetingId: Long, offset: Int, limit: Int): PhoneTranscriptPage?
    fun actionItems(meetingId: Long): List<PhoneActionItem>
    fun routineRuns(limit: Int): List<PhoneRoutineRun>
    fun prep(eventId: String): PhoneMeetingPrep?
    fun upcomingEvents(fromUnixMs: Long, untilUnixMs: Long, limit: Int): List<PhoneCalendarEvent>

    /** Topic pages as the computer built them; empty when none have synced. */
    fun topicSnapshots(): List<PhoneTopicSnapshot>

    /**
     * This replica's id for the meeting a topic page names (the recording device and its id there); null when
     * that meeting is not on this phone. Local ids differ per device, so a ref is never compared with one.
     */
    fun meetingId(deviceId: String, sourceMeetingId: Long): Long? = null
}

/** A capture's text for a citation. Separate from [PhoneReadStore] so the screens' fakes needn't know about captures. */
interface PhoneCaptureStore {
    fun captureItem(id: Long): PhoneCaptureItem?
}
