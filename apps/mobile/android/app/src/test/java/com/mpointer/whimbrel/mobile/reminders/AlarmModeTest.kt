package com.mpointer.whimbrel.mobile.reminders

import org.junit.Assert.assertEquals
import org.junit.Test

class AlarmModeTest {
    @Test
    fun exactWhenPermitted() {
        assertEquals(AlarmMode.Exact, alarmMode(sdk = 34, canScheduleExact = true))
    }

    @Test
    fun windowEndingAtFireTimeOtherwise() {
        assertEquals(AlarmMode.Window, alarmMode(sdk = 34, canScheduleExact = false))
        assertEquals(10 * 60_000L, ALARM_WINDOW_MS)
        assertEquals(1_000_000L - ALARM_WINDOW_MS, windowStart(fireAtMs = 1_000_000L))
    }

    @Test
    fun exactBelowApi31() {
        assertEquals(AlarmMode.Exact, alarmMode(sdk = 30, canScheduleExact = false))
        assertEquals(AlarmMode.Exact, alarmMode(sdk = 29, canScheduleExact = false))
    }
}
