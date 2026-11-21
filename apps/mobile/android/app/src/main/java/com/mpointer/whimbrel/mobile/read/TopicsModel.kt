package com.mpointer.whimbrel.mobile.read

import android.content.Context
import android.text.format.DateUtils
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/** The Topics tab: the topic pages the computer built, the next meeting soonest first, then by name. Ported from `TopicsModel.swift`. */
class TopicsModel(private val clock: () -> Long = System::currentTimeMillis) {
    private val _topics = MutableStateFlow<List<PhoneTopicSnapshot>>(emptyList())
    val topics: StateFlow<List<PhoneTopicSnapshot>> = _topics.asStateFlow()

    /** False until the first read finished, so the screen can tell "loading" from "nothing". */
    private val _loaded = MutableStateFlow(false)
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    suspend fun reload(sync: SyncModel) {
        _topics.value = sorted(sync.read { it.topicSnapshots() }.orEmpty(), clock())
        _loaded.value = true
    }

    companion object {
        /** Next meeting soonest, then name; a next meeting that has passed since the build counts as none. */
        fun sorted(topics: List<PhoneTopicSnapshot>, nowMs: Long): List<PhoneTopicSnapshot> =
            topics.sortedWith(compareBy({ it.upcomingMeeting(nowMs)?.startUnixMs ?: Long.MAX_VALUE }, { it.name.lowercase() }))
    }
}

/** One topic page, with its meeting refs resolved to this replica's meeting ids. Ported from `TopicPageModel`. */
class TopicPageModel(val key: String) {
    private val _snapshot = MutableStateFlow<PhoneTopicSnapshot?>(null)
    val snapshot: StateFlow<PhoneTopicSnapshot?> = _snapshot.asStateFlow()
    private val _meetingIds = MutableStateFlow<Map<PhoneMeetingRef, Long>>(emptyMap())
    val meetingIds: StateFlow<Map<PhoneMeetingRef, Long>> = _meetingIds.asStateFlow()
    private val _loaded = MutableStateFlow(false)
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    suspend fun reload(sync: SyncModel) {
        // No store open (not yet synced, or wiped) reads as a missing page, not as loading.
        val result = sync.read { store ->
            store.topicSnapshots().firstOrNull { it.topicKey == key }?.let { it to resolve(it, store) }
        }
        _snapshot.value = result?.first
        _meetingIds.value = result?.second.orEmpty()
        _loaded.value = true
    }

    companion object {
        /** The meetings a page's steps and recent items name that this phone has, by ref. */
        fun resolve(snap: PhoneTopicSnapshot, store: PhoneReadStore): Map<PhoneMeetingRef, Long> {
            val refs = (snap.youOwe + snap.owedToYou + snap.done).mapNotNull { it.meeting } + snap.recent.mapNotNull { it.meeting }
            return refs.distinct().mapNotNull { ref -> store.meetingId(ref.deviceId, ref.sourceMeetingId)?.let { ref to it } }.toMap()
        }
    }
}

/** The words and date styles [TopicText] needs: strings.xml and the phone's formats in the app, fixed text in tests. */
interface TopicStrings {
    val overdue: String
    /** "due Fri, Nov 14". */
    fun due(day: String): String
    /** "asked Nov 3". */
    fun asked(day: String): String
    /** "1 you". */
    fun you(n: Long): String
    /** "Built on Mike's MacBook · 5 minutes ago". */
    fun builtOn(computer: String, whenText: String): String
    /** "Fri, Nov 14". */
    fun day(date: LocalDate): String
    /** "Nov 3". */
    fun monthDay(ms: Long): String
    /** "Tue, Nov 18, 10:00"; the time follows the phone's 12/24-hour setting. */
    fun dayTime(ms: Long): String
    /** "5 minutes ago". */
    fun ago(thenMs: Long, nowMs: Long): String
}

class ResourceTopicStrings(private val context: Context, private val zone: ZoneId = ZoneId.systemDefault()) : TopicStrings {
    private val dayFlags = DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_ALL or DateUtils.FORMAT_NO_YEAR

    override val overdue get() = context.getString(R.string.topic_overdue)
    override fun due(day: String) = context.getString(R.string.topic_due, day)
    override fun asked(day: String) = context.getString(R.string.topic_asked, day)
    override fun you(n: Long) = context.getString(R.string.topic_you, n)
    override fun builtOn(computer: String, whenText: String) = context.getString(R.string.topic_built_on, computer, whenText)
    override fun day(date: LocalDate) = DateUtils.formatDateTime(context, date.atStartOfDay(zone).toInstant().toEpochMilli(), dayFlags)
    override fun monthDay(ms: Long) = DateUtils.formatDateTime(context, ms, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR)
    override fun dayTime(ms: Long) = DateUtils.formatDateTime(context, ms, dayFlags or DateUtils.FORMAT_SHOW_TIME)
    override fun ago(thenMs: Long, nowMs: Long) =
        if (nowMs - thenMs < DateUtils.MINUTE_IN_MILLIS) context.getString(R.string.topic_just_now)
        else DateUtils.getRelativeTimeSpanString(thenMs, nowMs, DateUtils.MINUTE_IN_MILLIS).toString()
}

/** How a topic page's dates and lines read on the phone (the iPhone's `TopicText`). */
object TopicText {
    /** The day for a "YYYY-MM-DD" string (the string itself if it does not parse). */
    fun dayText(iso: String, strings: TopicStrings): String = runCatching { LocalDate.parse(iso) }.getOrNull()?.let(strings::day) ?: iso

    /** ISO dates sort as text, so an unparseable one is compared the same way the iPhone does. */
    fun isOverdue(iso: String, today: LocalDate): Boolean = iso < today.toString()

    /** "due Fri, Nov 14", or "overdue" once the date has passed. */
    fun dueText(iso: String, today: LocalDate, strings: TopicStrings): String =
        if (isOverdue(iso, today)) strings.overdue else strings.due(dayText(iso, strings))

    /** The page's lead: "SOW due Fri, Nov 14", or the next step and when it is due. */
    fun lead(lead: PhoneTopicSnapshot.Lead, today: LocalDate, strings: TopicStrings): String = when (lead) {
        is PhoneTopicSnapshot.Lead.Dated -> "${lead.label} ${dayText(lead.date, strings)}"
        is PhoneTopicSnapshot.Lead.NextStep -> lead.dueDate?.let { "${lead.text} ${dueText(it, today, strings)}" } ?: lead.text
    }

    /** "1 you · 3 acme.com": the user first, then other attendees by email domain. */
    fun attendees(next: PhoneTopicSnapshot.NextMeeting, strings: TopicStrings): String =
        (listOfNotNull(next.me.takeIf { it > 0 }?.let(strings::you)) + next.attendeesByOrg.map { "${it.count} ${it.domain}" }).joinToString(" · ")

    /** "Built on Mike's MacBook · 5 minutes ago"; a build time ahead of this phone's clock reads as just now. */
    fun footer(snap: PhoneTopicSnapshot, nowMs: Long, strings: TopicStrings): String =
        strings.builtOn(snap.builtBy, strings.ago(minOf(snap.builtAtUnixMs, nowMs), nowMs))
}
