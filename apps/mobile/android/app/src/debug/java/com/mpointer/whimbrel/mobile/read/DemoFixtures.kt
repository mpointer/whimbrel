package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import com.mpointer.whimbrel.mobile.sync.EnrollmentBackend
import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import java.time.Instant
import java.time.ZoneId

/**
 * `whimbrel_demo_synced` read data (debug builds only): every screen can be shown without real data or a
 * server. Times are relative to [clock], so Today always has "this morning's" brief and a next meeting.
 * Ported from `DemoFixtures.swift`; the meeting list is the iPhone's, served directly rather than through a
 * real store.
 */
class DemoSyncStore(
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
) : MeetingSyncStore {
    private companion object {
        const val MINUTE = 60_000L
        const val HOUR = 3_600_000L
        const val DAY = 86_400_000L

        val titles = listOf(
            "Customer call: Acme pricing", "Design review", "Standup", "Planning", "1:1 with Sam", "Retro",
            "Partner sync with a very long title that has to wrap across several lines on a small phone screen", "Roadmap",
            "Hiring panel", "Board prep", "Weekly metrics", "Onboarding",
        )
        const val PREP_BRIEF = """{"summary":{"text":"Acme wants a quote for 200 seats; last call ended on discount tiers.","sources":[]},"attendees":[{"who":"Priya Nair","note":"Procurement lead, cares about annual terms","sources":[]}],"open_items":[{"text":"Send revised tier table","sources":[]}],"open_questions":[{"text":"Grandfather existing customers?","sources":[]}],"sources":[]}"""
    }

    /** Today's bounds in [zone], as Today reads them: the demo works just after midnight and late at night. */
    private val startOfToday get() = Instant.ofEpochMilli(clock()).atZone(zone).toLocalDate().atStartOfDay(zone)

    private fun meetings(): List<PhoneMeeting> = titles.mapIndexed { i, title ->
        val start = clock() - i * 5 * HOUR - (i / 3) * DAY / 2
        PhoneMeeting(
            id = i + 1L,
            deviceId = if (i % 3 == 1) "demo-mac-b" else "demo-mac-a",
            startUnixMs = start,
            endUnixMs = start + 30 * MINUTE,
            title = title,
            attendees = if (i % 2 == 0) listOf("Sam Rivera", "Ann Lee", "Priya Nair", "Tom Webb") else listOf("Sam Rivera"),
            summaryTldr = if (i == 2) null else "We agreed to ship the pricing change on Friday and revisit the discount tiers next week.",
            summaryDecisions = if (i == 2) emptyList() else listOf("Ship pricing change Friday", "Keep the annual discount at 15%"),
            summaryActionItems = if (i == 2) emptyList() else listOf("Sam to update the pricing page", "Ann to brief support"),
            summaryOpenQuestions = if (i == 2) emptyList() else listOf("Do we grandfather existing customers?"),
            alsoRecorded = if (i == 4) listOf(PhoneMeetingCopy(99, "demo-mac-b", title)) else null,
        )
    }

    override fun recentMeetings(limit: Int) = meetings().take(limit)
    override fun meeting(id: Long) = meetings().firstOrNull { it.id == id }

    override fun transcriptPage(meetingId: Long, offset: Int, limit: Int): PhoneTranscriptPage {
        if (meetingId != 1L) return PhoneTranscriptPage(emptyList(), 0)
        val all = (0 until 150).map { i ->
            PhoneTranscriptSegment(i.toLong(), i * 900L, if (i % 2 == 0) "Speaker 1" else "Me", "Line ${i + 1} of the call: we walked through the pricing options and the open questions.")
        }
        return PhoneTranscriptPage(all.drop(offset).take(limit), all.size)
    }

    override fun actionItems(meetingId: Long): List<PhoneActionItem> =
        if (meetingId == 3L) emptyList() // the meeting with no summary has none
        else listOf(PhoneActionItem(1, "Sam to update the pricing page", true, 0), PhoneActionItem(2, "Ann to brief support", false, 1))

    override fun routineRuns(limit: Int): List<PhoneRoutineRun> {
        val now = clock()
        return listOf(
            PhoneRoutineRun(
                1, "Daily Brief", maxOf(now - 30 * MINUTE, startOfToday.toInstant().toEpochMilli()), "success",
                "Today you have 3 meetings. The Acme pricing call at 10:30 is the one that matters: Sam owes the revised tiers, and support has not been briefed. Two follow-ups from yesterday are still open, including the contract redline Ann sent on Monday.",
                "demo-mac-a",
            ),
            PhoneRoutineRun(2, "Meeting Prep", now - 45 * MINUTE, "success", "Prepared 1 meeting.", null),
            PhoneRoutineRun(3, "Inbox triage", now - 3 * HOUR, "failed", "Gmail permission was revoked. Reconnect Gmail on your Mac.", "demo-mac-a"),
            PhoneRoutineRun(4, "Daily Brief", now - DAY, "success", "Yesterday's brief.", "demo-mac-a"),
        ).take(limit)
    }

    override fun prep(eventId: String): PhoneMeetingPrep? =
        if (eventId == "demo-next") PhoneMeetingPrep(eventId, "Customer call: Acme pricing", clock(), PREP_BRIEF) else null

    override fun upcomingEvents(fromUnixMs: Long, untilUnixMs: Long, limit: Int): List<PhoneCalendarEvent> {
        // 40 minutes out, or halfway to midnight when that is sooner.
        val now = clock()
        val endOfToday = startOfToday.plusDays(1).toInstant().toEpochMilli()
        val start = now + minOf(40 * MINUTE, (endOfToday - now) / 2)
        if (start < fromUnixMs || start >= untilUnixMs) return emptyList()
        return listOf(PhoneCalendarEvent("demo-next", "google_calendar", "Customer call: Acme pricing", start, listOf("Priya Nair", "Sam Rivera", "Ann Lee"))).take(limit)
    }

    override fun topicSnapshots(): List<PhoneTopicSnapshot> = emptyList()

    /** The "pull" has nothing to fetch: the fixtures are already here. */
    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int) = true
    override fun groupedMeetingCount(): Int = meetings().size
    override fun lastSyncError(): String? = null
    override fun moreToPull(): Boolean = false
    override fun close() {}
}

/** An approved account with a Mac and no Google or server, as the iPhone's `DemoSyncedBackend`. */
class DemoSyncedBackend : EnrollmentBackend {
    override suspend fun signIn() {}
    override suspend fun idToken() = "demo"
    override suspend fun authenticate(idToken: String, devicePublicKey: String) = GoogleAuthResult.Approved("demo", "demo", null)
    override suspend fun listDevices(idToken: String, devicePublicKey: String) =
        listOf(
            SyncDeviceRecord("mac", "demo", "demo-mac-a", "Mac", "macos", "approved", 0),
            SyncDeviceRecord("mac2", "demo", "demo-mac-b", "Mac mini", "macos", "approved", 0),
        )
    override suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) {}
    override suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) {}
    override suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) {}
    override suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) {}
    override suspend fun accountPlan(idToken: String, devicePublicKey: String) = AccountPlan("free")
    override fun signOut() {}
}
