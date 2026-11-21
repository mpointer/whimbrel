package com.mpointer.whimbrel.mobile.design

import androidx.compose.runtime.staticCompositionLocalOf

/**
 * The words the design-system components draw themselves, the same words as the desktop Thread system.
 * Components never read string resources: a screen fills this from resources and provides it through
 * [LocalWStrings], so the debug gallery and tests can use the English defaults.
 */
data class WStrings(
    val recording: String = "Recording",
    val paused: String = "Paused",
    val processing: String = "Processing",
    val notRecording: String = "Not recording",
    val problem: String = "Problem",
    val pause: String = "Pause",
    val resume: String = "Resume",
    val stop: String = "Stop",
    val stopAndSave: String = "Stop and save",
    val consentMessage: String = "You are recording. Tell the others before you go on.",
    val showNotice: String = "Show notice",
    val done: String = "Done",
    val notDone: String = "Not done",
    val searchPlaceholder: String = "Search your work memory",
    val now: String = "Now",
    val memory: String = "Memory",
    val transcriptMoment: String = "Transcript moment",
    val seenOnScreen: String = "Seen on screen",
    val open: String = "Open",
    val listening: String = "Listening",
    val cited: String = "Cited",
    val loading: String = "Loading",
    val tryAgain: String = "Try again",
    val offline: String = "No connection. Showing what is already on this phone.",
    val whimbrelWrote: (String) -> String = { "Whimbrel wrote: $it" },
    val sourceNumber: (Int) -> String = { "Source $it" },
    /** "Recording, 12 minutes 47 seconds": a state and a detail joined for speech. */
    val spoken: (String, String) -> String = { a, b -> "$a, $b" },
    /** "12 minutes 47 seconds", for TalkBack. */
    val elapsedSpoken: (hours: Int, minutes: Int, seconds: Int) -> String = { h, m, s ->
        listOfNotNull(
            if (h > 0) "$h ${if (h == 1) "hour" else "hours"}" else null,
            "$m ${if (m == 1) "minute" else "minutes"}",
            "$s ${if (s == 1) "second" else "seconds"}",
        ).joinToString(" ")
    },
)

val LocalWStrings = staticCompositionLocalOf { WStrings() }

/** Light or dark, and whether the system asks for more contrast. Provided once near the root. */
data class WAppearance(val dark: Boolean = false, val highContrast: Boolean = false)

val LocalWAppearance = staticCompositionLocalOf { WAppearance() }

object WElapsed {
    /** "12:47", or "1:02:47" past an hour. */
    fun clock(seconds: Int): String {
        val s = seconds.coerceAtLeast(0)
        return if (s >= 3600) "%d:%02d:%02d".format(s / 3600, (s % 3600) / 60, s % 60) else "%d:%02d".format(s / 60, s % 60)
    }

    fun spoken(seconds: Int, strings: WStrings): String {
        val s = seconds.coerceAtLeast(0)
        return strings.elapsedSpoken(s / 3600, (s % 3600) / 60, s % 60)
    }
}

/** What a recording is doing. Every state has its own glyph and word, so it never relies on color. */
sealed interface WRecordingState {
    data object Recording : WRecordingState
    data object Paused : WRecordingState
    data object Processing : WRecordingState
    data object Off : WRecordingState
    data class Problem(val reason: String) : WRecordingState

    val isLive: Boolean get() = this is Recording || this is Paused

    fun word(s: WStrings): String = when (this) {
        Recording -> s.recording
        Paused -> s.paused
        Processing -> s.processing
        Off -> s.notRecording
        is Problem -> s.problem
    }

    /** "Recording, 12 minutes 47 seconds" or "Problem, Microphone not found". */
    fun spokenLabel(elapsedSeconds: Int, s: WStrings): String = when {
        this is Problem -> s.spoken(s.problem, reason)
        isLive -> s.spoken(word(s), WElapsed.spoken(elapsedSeconds, s))
        else -> word(s)
    }
}

fun speakerInitials(name: String): String =
    name.split(" ").filter { it.isNotBlank() }.take(2).mapNotNull { it.firstOrNull()?.uppercaseChar() }.joinToString("")
