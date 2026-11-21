package com.mpointer.whimbrel.mobile.ui

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.ui.draw.clip
import com.mpointer.whimbrel.mobile.design.LocalWStrings
import com.mpointer.whimbrel.mobile.design.WAuthor
import com.mpointer.whimbrel.mobile.design.WNoteBlock
import com.mpointer.whimbrel.mobile.design.WNoteKind
import com.mpointer.whimbrel.mobile.design.reading
import android.text.format.DateUtils
import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.AccountCircle
import androidx.compose.material3.AssistChip
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Scaffold
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.produceState
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.R
import androidx.compose.ui.semantics.Role
import com.mpointer.whimbrel.mobile.design.WButton
import com.mpointer.whimbrel.mobile.design.WButtonKind
import com.mpointer.whimbrel.mobile.design.WCard
import com.mpointer.whimbrel.mobile.design.WMeetingCard
import com.mpointer.whimbrel.mobile.design.WMeetingVariant
import com.mpointer.whimbrel.mobile.design.WNowMarker
import com.mpointer.whimbrel.mobile.design.WSpineNode
import com.mpointer.whimbrel.mobile.design.WSpineStop
import com.mpointer.whimbrel.mobile.design.WTokens
import com.mpointer.whimbrel.mobile.design.style
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette
import com.mpointer.whimbrel.mobile.read.PhoneMeeting
import com.mpointer.whimbrel.mobile.read.PhoneMeetingPrep
import com.mpointer.whimbrel.mobile.read.ReadLogic
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.read.TodayModel
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.ZoneId
import kotlinx.coroutines.delay
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

private const val BRIEF_COLLAPSED_LINES = 4
private const val BRIEF_LONG_CHARS = 60
private val ContentMaxWidth = 640.dp

/** The sync status line, aged every 30 seconds while shown. */
@Composable
internal fun rememberSyncLine(sync: SyncModel): String {
    val syncing by sync.isSyncing.collectAsState()
    val lastAt by sync.lastSyncedAtMs.collectAsState()
    val error by sync.lastError.collectAsState()
    val context = LocalContext.current
    val now by produceState(System.currentTimeMillis()) { while (true) { delay(30_000); value = System.currentTimeMillis() } }
    return ReadLogic.syncLine(lastAt, now, syncing, error?.let { stringResource(it.resId) }, ResourceReadStrings(context))
}

/** Today's date and time-of-day text come from the platform's formatter, so they follow the phone's locale. */
private fun timeText(context: android.content.Context, ms: Long, withDate: Boolean) =
    DateUtils.formatDateTime(context, ms, DateUtils.FORMAT_SHOW_TIME or if (withDate) DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH else 0)

/** A past stop's label: the time, with the day above it when the meeting was not today, so the times down the thread never read as out of order. */
internal fun stopTime(context: android.content.Context, ms: Long): String {
    val time = timeText(context, ms, withDate = false)
    if (DateUtils.isToday(ms)) return time
    return DateUtils.formatDateTime(context, ms, DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR) + "\n" + time
}

/**
 * The phone's first screen, as one line of time: the next meeting with its prep above the now marker; the
 * day's brief and recent meetings behind it; then when the replica last synced. Pull down to sync. Ported from `TodayView.swift`.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TodayScreen(
    sync: SyncModel,
    model: TodayModel,
    labels: StateFlow<DeviceLabels>,
    onOpenSettings: () -> Unit,
    onOpenMeeting: (Long) -> Unit,
    /** Devices waiting for approval: an in-app banner that opens Settings (no push yet). */
    pendingDevices: Int = 0,
    /** Demo screenshots only: opens the prep sheet at launch. */
    startWithPrep: Boolean = false,
    update: com.mpointer.whimbrel.mobile.update.AvailableUpdate? = null,
    onDismissUpdate: () -> Unit = {},
) {
    val context = LocalContext.current
    val scope = rememberCoroutineScope() // Main: the models are main-thread only
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(revision) { model.reload(sync, System.currentTimeMillis(), ZoneId.systemDefault()) }

    val brief by model.brief.collectAsState()
    val upNext by model.upNext.collectAsState()
    val prep by model.prep.collectAsState()
    val recent by model.recent.collectAsState()
    val loaded by model.loaded.collectAsState()
    val deviceLabels by labels.collectAsState()
    val strings = remember(context) { ResourceReadStrings(context) }
    val syncLine = rememberSyncLine(sync)
    val olderLoading by sync.olderHistoryLoading.collectAsState()
    val updateNeeded by sync.updateNeeded.collectAsState()
    var refreshing by remember { mutableStateOf(false) }
    var briefExpanded by rememberSaveable { mutableStateOf(false) }
    var showPrep by rememberSaveable { mutableStateOf(startWithPrep) }
    val now = System.currentTimeMillis()
    val hour = java.time.Instant.ofEpochMilli(now).atZone(ZoneId.systemDefault()).hour
    val greeting = when (ReadLogic.daypart(hour)) {
        "morning" -> R.string.greeting_morning
        "afternoon" -> R.string.greeting_afternoon
        else -> R.string.greeting_evening
    }
    val loading = stringResource(R.string.today_loading)

    Scaffold(
        topBar = {
            TopAppBar(
                title = {},
                actions = {
                    IconButton(onClick = onOpenSettings) { Icon(Icons.Filled.AccountCircle, stringResource(R.string.settings_open)) }
                },
            )
        },
        contentWindowInsets = WindowInsets(0),
    ) { padding ->
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
            modifier = Modifier.padding(padding).fillMaxSize(),
        ) {
            // The inner screen would stretch a phone column; center it at a reading width.
            Box(Modifier.fillMaxSize().verticalScroll(rememberScrollState()), contentAlignment = Alignment.TopCenter) {
                Column(Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxWidth().padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl), verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                    Column(Modifier.padding(bottom = WTokens.Space.sm), verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
                        Text(stringResource(R.string.tab_today), style = WTokens.TypeScale.pageTitle.style(), modifier = Modifier.semantics { heading() })
                        Text(DateUtils.formatDateTime(context, now, DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    if (update != null) {
                        WCard(tint = LocalThemePalette.current.highlight) {
                            Text(stringResource(R.string.update_available, update.version), style = WTokens.TypeScale.subtitle.style())
                            Muted(stringResource(R.string.update_available_detail))
                            Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                                WButton(stringResource(R.string.update_download), {
                                    runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(update.apkUrl))) }
                                }, kind = WButtonKind.Quiet)
                                WButton(stringResource(R.string.update_not_now), onDismissUpdate, kind = WButtonKind.Quiet)
                            }
                        }
                    }
                    if (pendingDevices > 0) {
                        WCard(Modifier.clip(RoundedCornerShape(WTokens.Radius.card)).clickable(role = Role.Button, onClick = onOpenSettings), tint = LocalThemePalette.current.highlight) {
                            Text(pluralStringResource(R.plurals.devices_waiting, pendingDevices, pendingDevices), style = WTokens.TypeScale.body.style())
                        }
                    }
                    // What is coming: the next meeting, above the now marker.
                    val event = upNext
                    if (event != null) {
                        WSpineStop(timeText(context, event.startUnixMs, withDate = false), node = WSpineNode.Upcoming) {
                            WMeetingCard(
                                WMeetingVariant.Next, event.title,
                                status = listOfNotNull(stringResource(R.string.today_up_next), ReadLogic.relativeStart(event.startUnixMs, null, now, strings)).joinToString(" · "),
                                detail = if (event.attendees.isNotEmpty()) ReadLogic.attendeeSummary(event.attendees) else null,
                                actions = if (prep?.brief != null) { { WButton(stringResource(R.string.today_prep_ready), { showPrep = true }, kind = WButtonKind.Secondary) } } else null,
                            )
                        }
                    } else {
                        WSpineStop("", showsNode = false) { Muted(if (loaded) stringResource(R.string.today_nothing_else) else loading, Modifier.padding(WTokens.Space.md)) }
                    }
                    WNowMarker(timeText(context, now, withDate = false), Modifier.padding(vertical = WTokens.Space.xs))
                    // The day's brief, on the thread at the time it ran.
                    val run = brief
                    WSpineStop(run?.let { timeText(context, it.ranAtUnixMs, withDate = false) } ?: "", node = WSpineNode.Past, showsNode = run != null) {
                        WCard {
                            Text(stringResource(R.string.today_daily_brief), style = WTokens.TypeScale.subtitle.style(), modifier = Modifier.semantics { heading() })
                            val text = run?.outputSummary
                            if (text != null) {
                                Text(text, style = WTokens.TypeScale.body.style(), maxLines = if (briefExpanded) Int.MAX_VALUE else BRIEF_COLLAPSED_LINES, overflow = TextOverflow.Ellipsis)
                                if (text.length > BRIEF_LONG_CHARS) {
                                    WButton(stringResource(if (briefExpanded) R.string.today_show_less else R.string.today_show_more), { briefExpanded = !briefExpanded }, kind = WButtonKind.Quiet)
                                }
                            } else {
                                Muted(if (loaded) stringResource(R.string.today_no_brief) else loading)
                            }
                        }
                    }
                    // Meetings that have happened, newest first, each a stop behind the now marker.
                    if (recent.isEmpty()) {
                        WSpineStop("", node = WSpineNode.Past, showsNode = false) { Muted(if (loaded) stringResource(R.string.today_no_meetings) else loading, Modifier.padding(WTokens.Space.md)) }
                    }
                    recent.forEach { meeting ->
                        val row = meetingRowText(meeting, deviceLabels, strings, ZoneId.systemDefault(), withDate = false)
                        WSpineStop(stopTime(context, meeting.startUnixMs), node = WSpineNode.Past) {
                            WMeetingCard(
                                WMeetingVariant.Past, row.title, Modifier.clickable(role = Role.Button) { onOpenMeeting(meeting.id) },
                                detail = listOfNotNull(row.device, row.alsoRecorded, row.attendees.ifEmpty { null }).joinToString(" · "),
                            )
                        }
                    }
                    Text(syncLine, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
                    if (olderLoading) {
                        Text(stringResource(R.string.sync_older_history_loading), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
                    }
                    if (updateNeeded) {
                        Text(stringResource(R.string.sync_update_needed), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
                    }
                }
            }
        }
    }
    if (showPrep && prep?.brief != null) PrepSheet(prep, onDismiss = { showPrep = false })
}

@Composable
private fun SectionLabel(text: String) =
    Text(text.uppercase(), style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.semantics { heading() })

@Composable
private fun Muted(text: String, modifier: Modifier = Modifier) = Text(text, modifier, style = WTokens.TypeScale.body.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)

@Composable
private fun InfoCard(content: @Composable () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
    ) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { content() }
    }
}

/**
 * The synced prep for the next meeting: its summary, attendee notes and open items. Whimbrel wrote all of it, so
 * the sheet says so once and every line carries Whimbrel's ink and rule.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PrepSheet(prep: PhoneMeetingPrep?, onDismiss: () -> Unit) {
    val brief = prep?.brief
    val context = LocalContext.current
    // The legend says who wrote the lines, so TalkBack is not told before each one.
    val base = LocalWStrings.current
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = MaterialTheme.colorScheme.background) {
        CompositionLocalProvider(LocalWStrings provides remember(base) { base.copy(whimbrelWrote = { it }) }) {
            Column(
                Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl),
                verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
            ) {
                Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.Top, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                    Column(Modifier.weight(1f).semantics(mergeDescendants = true) { heading() }, verticalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
                        Text(prep?.title ?: stringResource(R.string.prep_title_fallback), style = WTokens.TypeScale.readingTitle.reading())
                        prep?.let { Text(timeText(context, it.startUnixMs, withDate = true), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant) }
                    }
                    WButton(stringResource(R.string.done), onDismiss, kind = WButtonKind.Quiet)
                }
                if (brief != null) {
                    WrittenByLegend()
                    WNoteBlock(WNoteKind.Paragraph, WAuthor.Whimbrel, brief.summary.text)
                    if (brief.attendees.isNotEmpty()) PrepGroup(stringResource(R.string.prep_attendees), brief.attendees.map { stringResource(R.string.prep_attendee_note, it.who, it.note) })
                    if (brief.openItems.isNotEmpty()) PrepGroup(stringResource(R.string.prep_open_items), brief.openItems.map { it.text })
                    if (brief.openQuestions.isNotEmpty()) PrepGroup(stringResource(R.string.prep_open_questions), brief.openQuestions.map { it.text })
                }
            }
        }
    }
}

@Composable
private fun PrepGroup(title: String, lines: List<String>) {
    Column(Modifier.padding(top = WTokens.Space.sm), verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
        WNoteBlock(WNoteKind.Heading, WAuthor.You, title)
        lines.forEach { WNoteBlock(WNoteKind.Bullet, WAuthor.Whimbrel, it) }
    }
}
