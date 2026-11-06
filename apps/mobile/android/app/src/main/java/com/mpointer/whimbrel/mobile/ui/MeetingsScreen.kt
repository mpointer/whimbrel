package com.mpointer.whimbrel.mobile.ui

import android.text.format.DateUtils
import androidx.compose.foundation.background
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.horizontalScroll
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilterChip
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.layout.AnimatedPane
import androidx.compose.material3.adaptive.layout.ListDetailPaneScaffoldRole
import androidx.compose.material3.adaptive.layout.PaneAdaptedValue
import androidx.compose.material3.adaptive.navigation.BackNavigationBehavior
import androidx.compose.material3.adaptive.navigation.NavigableListDetailPaneScaffold
import androidx.compose.material3.adaptive.navigation.ThreePaneScaffoldNavigator
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.read.MeetingsModel
import com.mpointer.whimbrel.mobile.read.PhoneMeeting
import com.mpointer.whimbrel.mobile.read.ReadLogic
import com.mpointer.whimbrel.mobile.read.ReadStrings
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import com.mpointer.whimbrel.mobile.sync.SyncModel
import java.time.LocalDate
import java.time.ZoneId
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

/** What a meeting row says, already formatted; the layout only wraps it. */
data class MeetingRow(val title: String, val timeRange: String, val attendees: String, val alsoRecorded: String?, val device: String)

/** A port of `MeetingRow` in `TodayView.swift`, plus the end time and attendees. [withDate] adds the day for lists that are not grouped by day. */
fun meetingRowText(meeting: PhoneMeeting, labels: DeviceLabels, strings: ReadStrings, zone: ZoneId, withDate: Boolean = false) = MeetingRow(
    title = meeting.title,
    timeRange = strings.timeRange(meeting.startUnixMs, meeting.endUnixMs, zone, withDate),
    attendees = ReadLogic.attendeeSummary(meeting.attendees),
    alsoRecorded = meeting.alsoRecorded?.size?.takeIf { it > 0 }?.let(strings::copies),
    device = strings.device(labels.label(meeting.deviceId)),
)

/** One meeting in a list: title, then time, where it was recorded, how many copies exist, and who was there. */
@Composable
fun MeetingListRow(meeting: PhoneMeeting, labels: DeviceLabels, strings: ReadStrings, modifier: Modifier = Modifier, withDate: Boolean = false) {
    val row = meetingRowText(meeting, labels, strings, ZoneId.systemDefault(), withDate)
    Column(modifier.fillMaxWidth().heightIn(min = 48.dp), verticalArrangement = Arrangement.Center) {
        Text(row.title, style = MaterialTheme.typography.bodyLarge)
        Text(listOfNotNull(row.timeRange, row.device, row.alsoRecorded).joinToString(" · "), color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (row.attendees.isNotEmpty()) Text(row.attendees, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

/** A new selection replaces the open meeting, so the history never grows past [List, Detail]. */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
suspend fun ThreePaneScaffoldNavigator<Any>.openMeeting(id: Long) {
    if (currentDestination?.pane == ListDetailPaneScaffoldRole.Detail) navigateBack(BackNavigationBehavior.PopLatest)
    navigateTo(ListDetailPaneScaffoldRole.Detail, id)
}

/**
 * Meetings: the day-grouped list beside the open meeting on the inner screen, one pane at a time on the cover
 * screen. The navigator (hoisted to the shell, so Today can open a meeting) holds the selection and is saved.
 */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
@Composable
fun MeetingsScreen(sync: SyncModel, model: MeetingsModel, labels: StateFlow<DeviceLabels>, navigator: ThreePaneScaffoldNavigator<Any>, openMeeting: (Long) -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(revision) { model.reload(sync) }
    val deviceLabels by labels.collectAsState()
    val open = navigator.currentDestination?.takeIf { it.pane == ListDetailPaneScaffoldRole.Detail }?.content as? Long
    // Back arrow only when the list is not beside the meeting.
    // Kept so the compact exit animation does not flash the placeholder once the selection is gone.
    var lastOpen by remember { mutableStateOf<Long?>(null) }
    LaunchedEffect(open) { if (open != null) lastOpen = open }
    val listHidden = navigator.scaffoldValue[ListDetailPaneScaffoldRole.List] != PaneAdaptedValue.Expanded

    // PopLatest: Back closes the open meeting (compact: back to the list; two panes: clears the selection).
    NavigableListDetailPaneScaffold(
        navigator = navigator,
        defaultBackBehavior = BackNavigationBehavior.PopLatest,
        listPane = {
            AnimatedPane {
                MeetingList(sync, model, deviceLabels, selected = open, onOpen = openMeeting)
            }
        },
        detailPane = {
            AnimatedPane {
                val shown = open ?: lastOpen.takeIf { listHidden }
                if (shown != null) {
                    MeetingDetailScreen(shown, sync, deviceLabels, onBack = if (listHidden) ({ scope.launch { navigator.navigateBack(BackNavigationBehavior.PopLatest) } }) else null)
                } else {
                    Box(Modifier.fillMaxSize(), contentAlignment = Alignment.Center) {
                        Text(stringResource(R.string.meetings_pick), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                }
            }
        },
    )
}

@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun MeetingList(sync: SyncModel, model: MeetingsModel, labels: DeviceLabels, selected: Long?, onOpen: (Long) -> Unit) {
    val scope = rememberCoroutineScope()
    val context = LocalContext.current
    val strings = remember(context) { ResourceReadStrings(context) }
    val meetings by model.meetings.collectAsState()
    val loaded by model.loaded.collectAsState()
    var refreshing by remember { mutableStateOf(false) }
    val zone = ZoneId.systemDefault()
    val sections = remember(meetings, model.deviceFilter, model.query) { MeetingsModel.sections(meetings, model.deviceFilter, model.query, zone) }
    val devices = model.devices

    Column(Modifier.fillMaxSize().statusBarsPadding()) {
        OutlinedTextField(
            value = model.query,
            onValueChange = { model.query = it },
            placeholder = { Text(stringResource(R.string.meetings_search)) },
            singleLine = true,
            modifier = Modifier.fillMaxWidth().padding(horizontal = 16.dp, vertical = 8.dp),
        )
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
            modifier = Modifier.fillMaxSize(),
        ) {
            LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = 16.dp, vertical = 8.dp)) {
                if (devices.size > 1) item(key = "devices") {
                    Row(Modifier.fillMaxWidth().horizontalScroll(rememberScrollState()).padding(bottom = 8.dp), horizontalArrangement = Arrangement.spacedBy(8.dp)) {
                        FilterChip(selected = model.deviceFilter == null, onClick = { model.deviceFilter = null }, label = { Text(stringResource(R.string.meetings_all)) })
                        devices.forEach { id ->
                            FilterChip(
                                selected = model.deviceFilter == id,
                                onClick = { model.deviceFilter = id },
                                label = { Text(strings.device(labels.label(id))) },
                            )
                        }
                    }
                }
                sections.forEach { section ->
                    item(key = "day-${section.day}") {
                        Text(
                            dayTitle(section.day, zone), style = MaterialTheme.typography.labelLarge, fontWeight = FontWeight.SemiBold,
                            color = MaterialTheme.colorScheme.onSurfaceVariant,
                            modifier = Modifier.padding(top = 12.dp, bottom = 4.dp).semantics { heading() },
                        )
                    }
                    items(section.meetings, key = { it.id }) { meeting ->
                        MeetingListRow(
                            meeting, labels, strings,
                            Modifier
                                .background(if (meeting.id == selected) MaterialTheme.colorScheme.primaryContainer else androidx.compose.ui.graphics.Color.Transparent)
                                .clickable { onOpen(meeting.id) }
                                .padding(horizontal = 8.dp),
                        )
                    }
                }
                if (sections.isEmpty()) item(key = "empty") {
                    Text(
                        stringResource(if (!loaded) R.string.today_loading else if (meetings.isEmpty()) R.string.meetings_none else R.string.meetings_no_match),
                        color = MaterialTheme.colorScheme.onSurfaceVariant,
                    )
                }
                if (model.isCapped) item(key = "capped") {
                    Text(
                        stringResource(R.string.meetings_capped, MeetingsModel.limit), style = MaterialTheme.typography.bodySmall,
                        color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 12.dp),
                    )
                }
            }
        }
    }
}

@Composable
private fun dayTitle(day: LocalDate, zone: ZoneId): String {
    val today = LocalDate.now(zone)
    if (day == today) return stringResource(R.string.meetings_today)
    if (day == today.minusDays(1)) return stringResource(R.string.meetings_yesterday)
    val flags = DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or
        if (day.year == today.year) DateUtils.FORMAT_NO_YEAR else DateUtils.FORMAT_SHOW_YEAR
    return DateUtils.formatDateTime(LocalContext.current, day.atStartOfDay(zone).toInstant().toEpochMilli(), flags)
}
