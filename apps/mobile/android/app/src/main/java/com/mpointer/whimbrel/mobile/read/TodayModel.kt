package com.mpointer.whimbrel.mobile.read

import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.Instant
import java.time.ZoneId
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow

/**
 * What Today shows, read from the replica on the store thread; ported from `TodayModel.swift`.
 * Call from the main thread.
 */
class TodayModel {
    private val _brief = MutableStateFlow<PhoneRoutineRun?>(null)
    val brief: StateFlow<PhoneRoutineRun?> = _brief.asStateFlow()
    private val _upNext = MutableStateFlow<PhoneCalendarEvent?>(null)
    val upNext: StateFlow<PhoneCalendarEvent?> = _upNext.asStateFlow()
    private val _prep = MutableStateFlow<PhoneMeetingPrep?>(null)
    val prep: StateFlow<PhoneMeetingPrep?> = _prep.asStateFlow()
    private val _recent = MutableStateFlow<List<PhoneMeeting>>(emptyList())
    val recent: StateFlow<List<PhoneMeeting>> = _recent.asStateFlow()
    private val _loaded = MutableStateFlow(false)

    /** False until the first read finished, so the screen can tell "loading" from "nothing". */
    val loaded: StateFlow<Boolean> = _loaded.asStateFlow()

    private class Snapshot(val runs: List<PhoneRoutineRun>, val event: PhoneCalendarEvent?, val prep: PhoneMeetingPrep?, val meetings: List<PhoneMeeting>)

    companion object {
        private const val DAILY_BRIEF = "Daily Brief" // the routine's stored name, not display text

        private fun startOfDay(nowMs: Long, zone: ZoneId) =
            Instant.ofEpochMilli(nowMs).atZone(zone).toLocalDate().atStartOfDay(zone)

        /** The newest successful Daily Brief run dated today, from any device. */
        fun briefRun(runs: List<PhoneRoutineRun>, nowMs: Long, zone: ZoneId): PhoneRoutineRun? {
            val start = startOfDay(nowMs, zone)
            val startMs = start.toInstant().toEpochMilli()
            val endMs = start.plusDays(1).toInstant().toEpochMilli()
            return runs
                .filter { it.routineName == DAILY_BRIEF && it.succeeded && it.ranAtUnixMs in startMs until endMs }
                .maxByOrNull { it.ranAtUnixMs }
        }
    }

    suspend fun reload(sync: SyncModel, nowMs: Long, zone: ZoneId) {
        // Like the Mac: an event that started up to 15 minutes ago still counts as next.
        val from = nowMs - 15 * 60_000
        val endOfToday = startOfDay(nowMs, zone).plusDays(1).toInstant().toEpochMilli()
        val snapshot = sync.read { store ->
            val event = store.upcomingEvents(from, endOfToday, 1).firstOrNull()
            Snapshot(store.routineRuns(50), event, event?.let { store.prep(it.externalId) }, store.recentMeetings(10))
        }
        _brief.value = snapshot?.let { briefRun(it.runs, nowMs, zone) }
        _upNext.value = snapshot?.event
        _prep.value = snapshot?.prep
        _recent.value = snapshot?.meetings.orEmpty()
        _loaded.value = true
    }
}
