package com.mpointer.whimbrel.mobile.design

import android.app.UiModeManager
import android.content.Context
import android.os.Build
import androidx.compose.foundation.background
import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawingPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import com.mpointer.whimbrel.mobile.theme.ExtraColors
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette
import com.mpointer.whimbrel.mobile.theme.ThemePalette
import com.mpointer.whimbrel.mobile.theme.ThemeToken
import com.mpointer.whimbrel.mobile.theme.toColorScheme

/** The generated Thread palette as a theme palette. */
fun threadPalette(dark: Boolean, highContrast: Boolean): ThemePalette {
    val values = when {
        dark && highContrast -> WThreadPalette.highContrastDark
        highContrast -> WThreadPalette.highContrastLight
        dark -> WThreadPalette.dark
        else -> WThreadPalette.light
    }
    return ThemeToken.entries.associateWith { values.getValue(it.json) }
}

/**
 * Every design-system component in every state, for debug builds only. Open it with
 * `adb shell am start -n <pkg>/.MainActivity --ez whimbrel_gallery true` (add `--ez whimbrel_gallery_hc true`
 * for high contrast, `--es whimbrel_gallery_section thread` to show one section). It reads no account and no store.
 */
@Composable
fun WGallery(forceHighContrast: Boolean = false, onlySection: String? = null) {
    val dark = isSystemInDarkTheme()
    val context = LocalContext.current
    val systemContrast = if (Build.VERSION.SDK_INT >= 34) (context.getSystemService(Context.UI_MODE_SERVICE) as UiModeManager).contrast > 0.5f else false
    val hc = forceHighContrast || systemContrast
    val palette = threadPalette(dark, hc)
    fun c(t: ThemeToken) = Color(0xFF000000.toInt() or palette.getValue(t))
    val extras = ExtraColors(c(ThemeToken.success), c(ThemeToken.warning), c(ThemeToken.link), c(ThemeToken.highlight), c(ThemeToken.selection), c(ThemeToken.accentSecondary), c(ThemeToken.accentHighlight))
    var done by remember { mutableStateOf(false) }
    var selectedSource by remember { mutableIntStateOf(3) }
    var filter by remember { mutableStateOf(true) }

    CompositionLocalProvider(LocalThemePalette provides extras, LocalWAppearance provides WAppearance(dark, hc)) {
        MaterialTheme(colorScheme = palette.toColorScheme(dark)) {
            Column(
                Modifier.fillMaxSize().background(MaterialTheme.colorScheme.background).safeDrawingPadding().verticalScroll(rememberScrollState())
                    .padding(WTokens.Space.lg).widthIn(max = WTokens.Size.contentMax),
                verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
            ) {
                @Composable fun section(id: String, title: String, content: @Composable () -> Unit) {
                    if (onlySection != null && onlySection != id) return
                    Text(title, Modifier.padding(top = WTokens.Space.md).semantics { heading() }, style = WTokens.TypeScale.title.style(), color = MaterialTheme.colorScheme.onBackground)
                    content()
                }
                section("recording", "Recording indicator") {
                    WRecordingIndicator(WRecordingState.Recording, 767, onPauseResume = {}, onStop = {})
                    WRecordingIndicator(WRecordingState.Paused, 767, onPauseResume = {}, onStop = {})
                    WRecordingIndicator(WRecordingState.Processing, 0)
                    WRecordingIndicator(WRecordingState.Problem("Microphone not found"), 0)
                    WRecordingIndicator(WRecordingState.Off, 0)
                    WRecordingIndicator(WRecordingState.Recording, 3767, pill = true)
                    WConsentBanner({}, {})
                }
                section("search", "Search and filters") {
                    WSearchBar({})
                    Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                        WFilterChip("People", filter, { filter = !filter })
                        WFilterChip("Apps", false, {})
                        WFilterChip("Dates", false, {})
                    }
                }
                section("thread", "Thread") {
                    WSpineStop("1:30 PM") { WMeetingCard(WMeetingVariant.Upcoming, "Design sync", detail = "with Dana Whitfield + 3 · in 3 hr 38 min") }
                    WSpineStop("10:00 AM", node = WSpineNode.Now) {
                        WMeetingCard(WMeetingVariant.Next, "Pricing review", status = "Next, in 8 minutes", detail = "Last met Tue, Nov 10. Brief ready.", attendees = listOf("Sam Okafor", "Priya Shah")) {
                            WButton("Open brief", {}, Modifier.weight(1f), WButtonKind.Secondary)
                            WButton("Record", {}, Modifier.weight(1f), WButtonKind.Recording)
                        }
                    }
                    WNowMarker("9:52 AM")
                    WSpineStop("22 min", node = WSpineNode.Past, showsNode = false) {
                        WMemoryBlock("9:30 to 9:52 AM", listOf(
                            WMemoryItem(WAppDot.Green, "Pricing v3.xlsx in Excel"), WMemoryItem(WAppDot.Blue, "1 email from Priya Shah"),
                            WMemoryItem(WAppDot.Purple, "#pricing in Slack"), WMemoryItem(WAppDot.Amber, "Renewal notes in Notes"),
                        ), device = "MacBook Pro")
                    }
                    WSpineStop("Mon", node = WSpineNode.Past) { WMeetingCard(WMeetingVariant.Past, "Northwind renewal call", detail = "Two years if the price holds. Quote due Thursday.") }
                }
                section("transcript", "Transcript") {
                    WTranscriptLine("06:12", "If the price holds, we can do two years.", speaker = "Jo, Northwind")
                    WTranscriptLine("18:25", "There are still a few open items on the questionnaire.", speaker = "Jo, Northwind", state = WLineState.Cited)
                    WTranscriptLine("12:40", "Let me check what that does to the margin and come back by Friday.", state = WLineState.Live)
                }
                section("notes", "Note blocks and sources") {
                    WNoteBlock(WNoteKind.Heading, WAuthor.You, "Your notes")
                    WNoteBlock(WNoteKind.Paragraph, WAuthor.You, "jo ok with 2 yr term if price holds")
                    WNoteBlock(WNoteKind.Heading, WAuthor.Whimbrel, "Renewal terms")
                    WNoteBlock(WNoteKind.Paragraph, WAuthor.Whimbrel, "Northwind will renew for two years if the price stays the same.", source = 1, selected = selectedSource == 1, onSource = { selectedSource = 1 })
                    WNoteBlock(WNoteKind.Bullet, WAuthor.Whimbrel, "The security questionnaire still has open items.", source = 3, selected = selectedSource == 3, onSource = { selectedSource = 3 })
                    WSourceCard(3, WSourceKind.TranscriptMoment, "Northwind renewal call", "18:25", {}, quote = "Jo, Northwind: “There are still a few open items on the questionnaire.”", selected = true)
                    WSourceCard(2, WSourceKind.SeenOnScreen, "Pricing v3.xlsx in Excel", "Today 9:41 AM", {})
                }
                section("actions", "Action items and buttons") {
                    WActionItemRow("Send the two-year renewal quote to Jo.", done, { done = !done }, owner = "Sam Okafor", due = "due Thu, Nov 19")
                    WActionItemRow("Answer the open items on the questionnaire.", true, {}, owner = "Dana Whitfield")
                    Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                        WButton("Primary", {}); WButton("Secondary", {}, kind = WButtonKind.Secondary); WButton("Quiet", {}, kind = WButtonKind.Quiet)
                    }
                    Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                        WButton("Delete", {}, kind = WButtonKind.Destructive); WButton("Record", {}, kind = WButtonKind.Recording); WButton("Disabled", {}, enabled = false)
                    }
                }
                section("states", "States") {
                    WOfflineBanner()
                    WCard { WStateView(WStateKind.Loading, "Loading") }
                    WCard { WStateView(WStateKind.Empty, "No meetings yet", help = "Meetings you record on your computer or this phone show up here.") }
                    WCard { WStateView(WStateKind.Error, "Could not load this note", help = "The note is on this phone but could not be read.", onAction = {}) }
                }
            }
        }
    }
}
