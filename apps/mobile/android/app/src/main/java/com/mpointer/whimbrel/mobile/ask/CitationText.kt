package com.mpointer.whimbrel.mobile.ask

/** Where a citation chip leads. */
sealed interface CitationTarget {
    data class Meeting(val id: Long) : CitationTarget
    data class Capture(val id: Long) : CitationTarget
    data class RoutineRun(val id: Long) : CitationTarget
    data object None : CitationTarget

    companion object {
        /**
         * Transcript and snapshot cards open their meeting. Cards the replica doesn't have, and kinds with no
         * screen on the phone (episodes, threads), go nowhere.
         */
        fun of(card: CitationCard): CitationTarget {
            if (!card.found) return None
            return when (card.kind) {
                "transcript", "snapshot" -> card.meetingId?.let(::Meeting) ?: None
                "capture" -> Capture(card.id)
                "routine_run" -> RoutineRun(card.id)
                else -> None
            }
        }
    }
}

object CitationText {
    /** A date within the last week is shown as its weekday ("Tue"), an older one as month and day ("Oct 7"). */
    fun isRecent(tsUnixMs: Long, nowMs: Long): Boolean = kotlin.math.abs(nowMs - tsUnixMs) < 7 * 86_400_000L

    /**
     * "<subtitle> · <device name> · <title> · <short date>", leaving out what is empty ("Screen · MacBook ·
     * Pricing.xlsx"). A card that wasn't found keeps its kind ("Screen · Not on this phone").
     */
    fun label(card: CitationCard, deviceName: String?, notHere: String, shortDate: (Long) -> String): String {
        if (!card.found) return listOf(card.subtitle, notHere).filter { it.isNotEmpty() }.joinToString(" · ")
        return (listOf(card.subtitle, deviceName.orEmpty(), card.title) + listOfNotNull(card.tsUnixMs?.let(shortDate)))
            .filter { it.isNotEmpty() }.joinToString(" · ")
    }
}
