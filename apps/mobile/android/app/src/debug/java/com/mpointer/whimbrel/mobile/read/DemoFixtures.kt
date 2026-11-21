package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.ask.AskBackend
import com.mpointer.whimbrel.mobile.ask.AskModel
import com.mpointer.whimbrel.mobile.ask.AskTurn
import com.mpointer.whimbrel.mobile.ask.CitationCard
import com.mpointer.whimbrel.mobile.ask.InferenceEvent
import com.mpointer.whimbrel.mobile.ask.PhoneRoutingPrefs
import com.mpointer.whimbrel.mobile.ask.PrepareResult
import com.mpointer.whimbrel.mobile.core.AccountPlan
import com.mpointer.whimbrel.mobile.core.GoogleAuthResult
import com.mpointer.whimbrel.mobile.core.MeetingSyncStore
import com.mpointer.whimbrel.mobile.core.SyncDeviceRecord
import com.mpointer.whimbrel.mobile.sync.AskProvider
import com.mpointer.whimbrel.mobile.sync.EnrollmentBackend
import com.mpointer.whimbrel.mobile.sync.PrefStore
import com.mpointer.whimbrel.mobile.sync.SyncCredentials
import java.time.Instant
import java.time.ZoneId
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.flow.Flow
import kotlinx.coroutines.flow.emptyFlow
import kotlinx.serialization.json.add
import kotlinx.serialization.json.buildJsonArray
import kotlinx.serialization.json.buildJsonObject
import kotlinx.serialization.json.put

/**
 * `whimbrel_demo_synced` read data (debug builds only): every screen can be shown without real data or a
 * server. Times are relative to [clock], so Today always has "this morning's" brief and a next meeting.
 * Ported from `DemoFixtures.swift`; the meeting list is the iPhone's, served directly rather than through a
 * real store.
 */
class DemoSyncStore(
    private val clock: () -> Long = System::currentTimeMillis,
    private val zone: ZoneId = ZoneId.systemDefault(),
    /** No topics, so the Topics tab shows its first-run state. */
    private val empty: Boolean = false,
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

    private fun isoDay(offset: Long) = Instant.ofEpochMilli(clock()).atZone(zone).toLocalDate().plusDays(offset).toString()

    /** Three topic pages, as the iPhone's demo: a full one, one with a single step, and a bare one. */
    override fun topicSnapshots(): List<PhoneTopicSnapshot> {
        if (empty) return emptyList()
        val now = clock()
        val call = PhoneMeetingRef("demo-mac-a", 1)
        return listOf(
            PhoneTopicSnapshot(
                topicKey = "acme", name = "Acme renewal", kind = "customer",
                leadJson = buildJsonObject { put("kind", "date"); put("label", "Quote due"); put("date", isoDay(3)) },
                role = PhoneTopicSnapshot.Role("You own the quote and the discount decision.", true),
                nextMeeting = PhoneTopicSnapshot.NextMeeting(
                    "Customer call: Acme pricing", now + 2 * DAY,
                    buildJsonArray { add(buildJsonArray { add("acme.com"); add(3) }) }, me = 1,
                ),
                youOwe = listOf(
                    PhoneTopicSnapshot.Step("Send the revised tier table to Priya", isoDay(-1), "meeting", "Customer call: Acme pricing", now - 2 * DAY, call),
                    PhoneTopicSnapshot.Step("Confirm whether existing customers keep their price", isoDay(3), "email", "Re: renewal terms", now - DAY),
                ),
                owedToYou = listOf(PhoneTopicSnapshot.Step("Priya to send the signed order form", null, "email", "Order form", now - 3 * DAY)),
                done = listOf(PhoneTopicSnapshot.Step("Share the pricing deck", null, "meeting", "Design review")),
                recent = listOf(
                    PhoneTopicSnapshot.Recent("email", "Re: renewal terms", "Priya Nair", now - DAY),
                    PhoneTopicSnapshot.Recent("meeting", "Customer call: Acme pricing", null, now - 2 * DAY),
                    PhoneTopicSnapshot.Recent("document", "Pricing v3.xlsx", null, now - 4 * DAY),
                ),
                recentTotal = 9, truncated = true, builtAtUnixMs = now - 5 * MINUTE, builtBy = "Demo MacBook",
            ),
            PhoneTopicSnapshot(
                topicKey = "hiring", name = "Hiring: senior designer", kind = "project",
                leadJson = buildJsonObject { put("kind", "step"); put("text", "Send feedback on the two finalists"); put("due_date", isoDay(1)) },
                youOwe = listOf(PhoneTopicSnapshot.Step("Send feedback on the two finalists", isoDay(1), "meeting", "Hiring panel")),
                recent = listOf(PhoneTopicSnapshot.Recent("meeting", "Hiring panel", null, now - 3 * DAY)),
                recentTotal = 1, builtAtUnixMs = now - 2 * HOUR, builtBy = "Demo MacBook",
            ),
            PhoneTopicSnapshot(topicKey = "offsite", name = "Team offsite", builtAtUnixMs = now - DAY, builtBy = "Demo MacBook"),
        )
    }

    /** Only the Acme call's ref resolves, to the first demo meeting. */
    override fun meetingId(deviceId: String, sourceMeetingId: Long): Long? = if (deviceId == "demo-mac-a" && sourceMeetingId == 1L) 1L else null

    /** The "pull" has nothing to fetch: the fixtures are already here. */
    override fun pullOnly(c: SyncCredentials, captureWindowDays: Int) = true
    override fun groupedMeetingCount(): Int = meetings().size
    override fun lastSyncError(): String? = null
    override fun moreToPull(): Boolean = false
    override fun close() {}
}

/** An approved account with a Mac and no Google or server, as the iPhone's `DemoSyncedBackend`. */
class DemoSyncedBackend(
    /** A laptop waiting for approval, and two more that show one code, so the "don't approve" warning can be seen. */
    private val pendingDevices: Boolean = false,
) : EnrollmentBackend {
    private val pendingA by lazy { demoPublicKey() }
    private val pendingB by lazy { demoPublicKey() }

    override suspend fun signIn() {}
    override suspend fun idToken() = "demo"
    override suspend fun authenticate(idToken: String, devicePublicKey: String) = GoogleAuthResult.Approved("demo", "demo", null)
    override suspend fun listDevices(idToken: String, devicePublicKey: String) =
        listOf(
            SyncDeviceRecord("mac", "demo", "demo-mac-a", "Mac", "macos", "approved", 0),
            SyncDeviceRecord("mac2", "demo", "demo-mac-b", "Mac mini", "macos", "approved", 0),
        ) + if (!pendingDevices) emptyList() else listOf(
            SyncDeviceRecord("laptop", "demo", pendingA, "Work laptop", "windows", "pending", 0),
            SyncDeviceRecord("twin1", "demo", pendingB, "Studio Mac", "macos", "pending", 0),
            SyncDeviceRecord("twin2", "demo", pendingB, null, "macos", "pending", 0),
        )
    override suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) {}
    override suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) {}
    /** The demo approves nothing and reaches no server. */
    override suspend fun approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) {}
    override suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) {}
    override suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) {}
    override suspend fun accountPlan(idToken: String, devicePublicKey: String) = AccountPlan("free")
    override fun signOut() {}
}

/**
 * The demo's Ask: nothing here reaches a provider. A typed question fails at [prepare], so no prompt is built and
 * no stream starts. Ported from the iPhone's `DemoAskBackend`.
 */
class DemoAskBackend : AskBackend {
    override suspend fun prepare(query: String, historyJson: String, routingJson: String): PrepareResult = PrepareResult.Failed
    override suspend fun finalize(prepareId: String, text: String, model: String, inputTokens: Long, outputTokens: Long): List<CitationCard>? = null
    override fun discard(prepareId: String) {}
    override fun stream(prompt: String, model: String, provider: AskProvider, key: String): Flow<InferenceEvent> = emptyFlow()
    override fun resolveModel(provider: AskProvider, stored: String?): String = "claude-sonnet"
}

/** Settings that were never synced: every routing field unset. */
private class NoPrefs : PrefStore {
    override fun getString(key: String): String? = null
    override fun getLong(key: String): Long? = null
    override fun getBoolean(key: String): Boolean = false
    override fun putString(key: String, v: String) {}
    override fun putLong(key: String, v: Long) {}
    override fun putBoolean(key: String, v: Boolean) {}
    override fun clearPrefix(prefix: String) {}
}

object DemoAsk {
    /** A finished answer with its sources, a stopped one and a failed one; with [empty], no key and no turns. */
    fun model(scope: CoroutineScope, empty: Boolean, clock: () -> Long = System::currentTimeMillis): AskModel {
        // The "key" is a placeholder held in memory only; the real vault is never read or written.
        val model = AskModel(DemoAskBackend(), PhoneRoutingPrefs(NoPrefs()), { if (it == AskProvider.ANTHROPIC && !empty) "demo" else null }, scope)
        if (empty) return model
        val now = clock()
        model.seedDemo(
            listOf(
                AskTurn(
                    1, "What did we decide about Acme pricing?", "claude-sonnet",
                    "You agreed to ship the pricing change on Friday and keep the annual discount at 15%.\n\nOne thing is still open: whether existing customers keep their current price. Sam owes the updated pricing page, and Ann is briefing support.",
                    AskTurn.State.Done,
                    listOf(
                        CitationCard("transcript", 1, "Customer call: Acme pricing", "Meeting", now - 3_600_000, 1, "demo-mac-a", true),
                        CitationCard("capture", 7, "Pricing v3.xlsx", "Screen", now - 2 * 86_400_000L, null, "demo-mac-a", true),
                        CitationCard("capture", 8, "", "Screen", null, null, null, false),
                    ),
                ),
                AskTurn(2, "Who is briefing support?", "claude-sonnet", "Ann is briefing support before", AskTurn.State.Stopped),
                AskTurn(3, "And the hiring panel?", "claude-sonnet", state = AskTurn.State.Failed),
            ),
        )
        return model
    }
}

/** A real public key from a throwaway keypair, so the demo's pending devices show real comparison codes. */
private fun demoPublicKey(): String =
    kotlinx.serialization.json.Json.parseToJsonElement(com.mpointer.whimbrel.mobile.core.Native.syncGenerateDeviceKeypair()!!)
        .let { (it as kotlinx.serialization.json.JsonObject).getValue("public_key_base64") }
        .let { (it as kotlinx.serialization.json.JsonPrimitive).content }

/** `whimbrel_demo_limit`: sign-in answers that the free plan's phone slot is taken by another phone. */
class DemoLimitBackend : EnrollmentBackend {
    override suspend fun signIn() {}
    override suspend fun idToken() = "demo"
    override suspend fun authenticate(idToken: String, devicePublicKey: String): GoogleAuthResult =
        GoogleAuthResult.DeviceLimit("phone", listOf(com.mpointer.whimbrel.mobile.core.LimitHolder("other-phone", "Pixel 9", "android")))
    override suspend fun listDevices(idToken: String, devicePublicKey: String) = emptyList<SyncDeviceRecord>()
    override suspend fun revokeDevice(idToken: String, devicePublicKey: String, targetDeviceId: String) {}
    override suspend fun setDeviceName(idToken: String, devicePublicKey: String, name: String) {}
    override suspend fun approve(idToken: String, devicePublicKey: String, targetDeviceId: String, wrappedGroupKey: String) {}
    override suspend fun swapDevice(idToken: String, devicePublicKey: String, replaceDeviceId: String) {}
    override suspend fun joinWaitlist(idToken: String, devicePublicKey: String, wants: List<String>) {}
    override suspend fun accountPlan(idToken: String, devicePublicKey: String) = AccountPlan("free")
    override fun signOut() {}
}
