package com.mpointer.whimbrel.mobile.ui

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

/**
 * The phone's first screen: greeting, the day's brief, the next meeting with its prep, recent meetings and
 * when the replica last synced. Pull down to sync. Ported from `TodayView.swift`.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TodayScreen(
    sync: SyncModel,
    model: TodayModel,
    labels: StateFlow<DeviceLabels>,
    onOpenSettings: () -> Unit,
    onOpenMeeting: (Long) -> Unit,
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
    var refreshing by remember { mutableStateOf(false) }
    var briefExpanded by rememberSaveable { mutableStateOf(false) }
    var showPrep by rememberSaveable { mutableStateOf(false) }
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
                Column(Modifier.widthIn(max = ContentMaxWidth).fillMaxWidth().padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(20.dp)) {
                    Column(verticalArrangement = Arrangement.spacedBy(4.dp)) {
                        Text(DateUtils.formatDateTime(context, now, DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_NO_YEAR), style = MaterialTheme.typography.titleSmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(stringResource(greeting), style = MaterialTheme.typography.headlineLarge, fontFamily = FontFamily.Serif, modifier = Modifier.semantics { heading() })
                    }
                    if (update != null) {
                        InfoCard {
                            Text(stringResource(R.string.update_available, update.version), style = MaterialTheme.typography.titleMedium)
                            Muted(stringResource(R.string.update_available_detail))
                            Row(horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                                TextButton(onClick = {
                                    runCatching { context.startActivity(android.content.Intent(android.content.Intent.ACTION_VIEW, android.net.Uri.parse(update.pageUrl))) }
                                }) { Text(stringResource(R.string.update_download)) }
                                TextButton(onClick = onDismissUpdate) { Text(stringResource(R.string.update_not_now)) }
                            }
                        }
                    }
                    InfoCard {
                        SectionLabel(stringResource(R.string.today_daily_brief))
                        val text = brief?.outputSummary
                        if (text != null) {
                            Text(text, maxLines = if (briefExpanded) Int.MAX_VALUE else BRIEF_COLLAPSED_LINES, overflow = TextOverflow.Ellipsis)
                            if (text.length > BRIEF_LONG_CHARS) {
                                TextButton(onClick = { briefExpanded = !briefExpanded }) {
                                    Text(stringResource(if (briefExpanded) R.string.today_show_less else R.string.today_show_more))
                                }
                            }
                        } else {
                            Muted(if (loaded) stringResource(R.string.today_no_brief) else loading)
                        }
                    }
                    InfoCard {
                        SectionLabel(stringResource(R.string.today_up_next))
                        val event = upNext
                        if (event != null) {
                            Text(
                                timeText(context, event.startUnixMs, withDate = false),
                                style = MaterialTheme.typography.titleSmall, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.primary,
                            )
                            Text(event.title, style = MaterialTheme.typography.titleMedium)
                            if (event.attendees.isNotEmpty()) Muted(ReadLogic.attendeeSummary(event.attendees))
                            if (prep?.brief != null) {
                                AssistChip(onClick = { showPrep = true }, label = { Text(stringResource(R.string.today_prep_ready)) })
                            }
                        } else {
                            Muted(if (loaded) stringResource(R.string.today_nothing_else) else loading)
                        }
                    }
                    Column(verticalArrangement = Arrangement.spacedBy(8.dp)) {
                        SectionLabel(stringResource(R.string.today_recent_meetings))
                        if (recent.isEmpty()) Muted(if (loaded) stringResource(R.string.today_no_meetings) else loading)
                        recent.forEach { MeetingListRow(it, deviceLabels, strings, Modifier.clickable { onOpenMeeting(it.id) }, withDate = true) }
                    }
                    Text(syncLine, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, textAlign = TextAlign.Center, modifier = Modifier.fillMaxWidth())
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
private fun Muted(text: String) = Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant)

@Composable
private fun InfoCard(content: @Composable () -> Unit) {
    Card(
        modifier = Modifier.fillMaxWidth(),
        border = BorderStroke(1.dp, MaterialTheme.colorScheme.outline),
    ) {
        Column(Modifier.fillMaxWidth().padding(16.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) { content() }
    }
}

/** The synced prep for the next meeting: its summary, attendee notes and open items. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun PrepSheet(prep: PhoneMeetingPrep?, onDismiss: () -> Unit) {
    val brief = prep?.brief
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(16.dp)) {
            Text(prep?.title ?: stringResource(R.string.prep_title_fallback), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
            if (brief != null) {
                Text(brief.summary.text)
                if (brief.attendees.isNotEmpty()) PrepGroup(stringResource(R.string.prep_attendees), brief.attendees.map { stringResource(R.string.prep_attendee_note, it.who, it.note) })
                if (brief.openItems.isNotEmpty()) PrepGroup(stringResource(R.string.prep_open_items), brief.openItems.map { it.text })
                if (brief.openQuestions.isNotEmpty()) PrepGroup(stringResource(R.string.prep_open_questions), brief.openQuestions.map { it.text })
            }
            TextButton(onClick = onDismiss, modifier = Modifier.align(Alignment.End)) { Text(stringResource(R.string.done)) }
        }
    }
}

@Composable
private fun PrepGroup(title: String, lines: List<String>) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        SectionLabel(title)
        lines.forEach { Text("• $it") }
    }
}
