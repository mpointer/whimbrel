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
}
