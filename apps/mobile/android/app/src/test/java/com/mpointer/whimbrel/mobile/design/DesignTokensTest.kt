package com.mpointer.whimbrel.mobile.design

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertTrue
import org.junit.Test

class DesignTokensTest {
    @Test
    fun fixedColorPicksTheVariantForEachAppearance() {
        val c = WFixedColor(light = 1, dark = 2, highContrastLight = 3, highContrastDark = 4)
        assertEquals(1, c.value(dark = false, highContrast = false))
        assertEquals(2, c.value(dark = true, highContrast = false))
        assertEquals(3, c.value(dark = false, highContrast = true))
        assertEquals(4, c.value(dark = true, highContrast = true))
    }

    @Test
    fun recordingAndAiInkAreGenerated() {
        for (name in listOf("recording", "onRecording", "aiInk", "focus")) assertNotNull(name, WFixed.all[name])
    }

    @Test
    fun tapTargetMeetsTheMinimum() {
        assertTrue(WTokens.Size.target.value >= 48f)
    }

    @Test
    fun noTypeRoleIsUnderTheMinimumReadableSize() {
        val s = WTokens.TypeScale
        for (role in listOf(s.caption, s.secondary, s.body, s.bodyStrong, s.time, s.subtitle, s.title, s.pageTitle, s.reading, s.readingTitle)) {
            assertTrue(role.size.value >= s.minimumReadable.value)
            assertTrue(role.lineHeight.value >= role.size.value)
        }
    }
}

class DesignComponentsTest {
    private val s = WStrings()

    @Test
    fun elapsedClock() {
        assertEquals("0:00", WElapsed.clock(0))
        assertEquals("12:47", WElapsed.clock(767))
        assertEquals("1:02:47", WElapsed.clock(3767))
        assertEquals("0:00", WElapsed.clock(-5))
    }

    /** The spoken name carries the state in words, never color alone. */
    @Test
    fun recordingSpokenLabelNamesTheState() {
        assertEquals("Recording, 12 minutes 47 seconds", WRecordingState.Recording.spokenLabel(767, s))
        assertEquals("Paused, 1 hour 2 minutes 47 seconds", WRecordingState.Paused.spokenLabel(3767, s))
        assertEquals("Not recording", WRecordingState.Off.spokenLabel(0, s))
        assertEquals("Problem, Microphone not found", WRecordingState.Problem("Microphone not found").spokenLabel(0, s))
    }

    @Test
    fun everyRecordingStateHasItsOwnWord() {
        val states = listOf(WRecordingState.Recording, WRecordingState.Paused, WRecordingState.Processing, WRecordingState.Off, WRecordingState.Problem("x"))
        assertEquals(states.size, states.map { it.word(s) }.toSet().size)
    }

    @Test
    fun speakerInitialsTakeTwoWords() {
        assertEquals("SO", speakerInitials("Sam Okafor"))
        assertEquals("JN", speakerInitials("Jo, Northwind"))
        assertEquals("Y", speakerInitials("You"))
    }

    /** The consent wording matches desktop and never claims anyone agreed. */
    @Test
    fun consentWording() {
        assertEquals("You are recording. Tell the others before you go on.", s.consentMessage)
    }

    @Test
    fun threadPaletteHasEveryTokenInEveryMode() {
        val keys = com.mpointer.whimbrel.mobile.theme.ThemeToken.entries.map { it.json }.toSet()
        for (mode in listOf(WThreadPalette.light, WThreadPalette.dark, WThreadPalette.highContrastLight, WThreadPalette.highContrastDark)) assertEquals(keys, mode.keys)
    }

    @Test
    fun aThreadTimeWrapsBetweenTheTimeAndItsPeriodNotInsideIt() {
        org.junit.Assert.assertEquals("10:00 PM", spineTimeLabel("10:00\u202FPM"))
        org.junit.Assert.assertEquals("10:00 PM", spineTimeLabel("10:00\u00A0PM"))
        org.junit.Assert.assertEquals("a 24-hour time is left alone", "22:00", spineTimeLabel("22:00"))
        org.junit.Assert.assertEquals("Nov 18\n2:00 PM", spineTimeLabel("Nov 18\n2:00\u202FPM"))
    }
}
