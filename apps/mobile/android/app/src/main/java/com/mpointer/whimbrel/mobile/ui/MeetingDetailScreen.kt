package com.mpointer.whimbrel.mobile.ui

import android.text.format.DateUtils
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.SegmentedButton
import androidx.compose.material3.SegmentedButtonDefaults
import androidx.compose.material3.SingleChoiceSegmentedButtonRow
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.material3.TopAppBar
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.read.MeetingDetailModel
import com.mpointer.whimbrel.mobile.read.PhoneMeeting
import com.mpointer.whimbrel.mobile.read.ReadLogic
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import com.mpointer.whimbrel.mobile.sync.SyncModel
import kotlinx.coroutines.launch

private val SUMMARY = 0
private val TRANSCRIPT = 1

/** `m:ss` for a transcript position in centiseconds. */
internal fun transcriptTime(centiseconds: Long): String {
    val seconds = centiseconds / 100
    return "%d:%02d".format(seconds / 60, seconds % 60)
}

/** One meeting: where and when it was recorded, then a Summary and a Transcript view. Ported from `MeetingDetailView.swift`. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun MeetingDetailScreen(meetingId: Long, sync: SyncModel, labels: DeviceLabels, onBack: (() -> Unit)?) {
    val scope = rememberCoroutineScope() // Main
    val model = remember(meetingId) { MeetingDetailModel(meetingId) }
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(model, revision) { model.reload(sync) }
    val meeting by model.meeting.collectAsState()
    val items by model.actionItems.collectAsState()
    val segments by model.transcript.collectAsState()
    val total by model.transcriptTotal.collectAsState()
    val loaded by model.loaded.collectAsState()
    var tab by rememberSaveable(meetingId) { mutableIntStateOf(SUMMARY) }

    Column(Modifier.fillMaxSize()) {
        if (onBack != null) {
            TopAppBar(title = {}, navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) } })
        } else {
            Box(Modifier.statusBarsPadding())
        }
        LazyColumn(Modifier.fillMaxSize(), contentPadding = androidx.compose.foundation.layout.PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            val m = meeting
            if (m == null) {
                item { Muted(stringResource(if (loaded) R.string.detail_not_here else R.string.today_loading)) }
                return@LazyColumn
            }
            item(key = "header") { Header(m, labels) }
            if (m.attendees.isNotEmpty()) item(key = "participants") { Participants(m) }
            item(key = "picker") {
                val names = listOf(R.string.detail_summary, R.string.detail_transcript)
                val section = stringResource(R.string.detail_section)
                SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().semantics { contentDescription = section }) {
                    names.forEachIndexed { i, name ->
                        SegmentedButton(selected = tab == i, onClick = { tab = i }, shape = SegmentedButtonDefaults.itemShape(i, names.size), label = { Text(stringResource(name)) })
                    }
                }
            }
            if (tab == SUMMARY) {
                summary(m, items.map { it.text to it.completed })
            } else if (loaded && total == 0) {
                item { Muted(stringResource(R.string.detail_no_transcript)) }
            } else {
                items(segments, key = { it.id }) { segment ->
                    Column(Modifier.fillMaxWidth().semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(2.dp)) {
                        Text("${segment.speakerLabel} · ${transcriptTime(segment.startCs)}", style = MaterialTheme.typography.labelMedium, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(segment.text)
                    }
                }
                if (model.hasMoreTranscript) item(key = "more") {
                    TextButton(onClick = { scope.launch { model.loadMoreTranscript(sync) } }, modifier = Modifier.fillMaxWidth().heightIn(min = 48.dp)) {
                        Text(stringResource(R.string.detail_load_more))
                    }
                }
            }
        }
    }
}

@Composable
private fun Muted(text: String) = Text(text, color = MaterialTheme.colorScheme.onSurfaceVariant)

@Composable
private fun Header(meeting: PhoneMeeting, labels: DeviceLabels) {
    val context = LocalContext.current
    val strings = remember(context) { ResourceReadStrings(context) }
    val where = buildList {
        add(stringResource(R.string.detail_recorded_on, strings.device(labels.label(meeting.deviceId))))
        meeting.alsoRecorded?.takeIf { it.isNotEmpty() }?.let { copies ->
            add(stringResource(R.string.detail_also_on, copies.joinToString(", ") { strings.device(labels.label(it.deviceId)) }))
        }
    }.joinToString(" · ")
    val whenText = DateUtils.formatDateTime(
        context, meeting.startUnixMs,
        DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_SHOW_TIME or DateUtils.FORMAT_NO_YEAR,
    )
    Column(Modifier.semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(meeting.title, style = MaterialTheme.typography.headlineSmall, fontFamily = FontFamily.Serif, modifier = Modifier.semantics { heading() })
        Muted(whenText)
        Muted(where)
    }
}

/** The participants: three names and a "+N more" toggle until expanded; reset per meeting. */
@Composable
private fun Participants(meeting: PhoneMeeting) {
    var expanded by rememberSaveable(meeting.id) { mutableStateOf(false) }
    val hidden = ReadLogic.hiddenAttendeeCount(meeting.attendees)
    Column {
        Muted(ReadLogic.attendeeLine(meeting.attendees, expanded))
        if (hidden > 0) {
            val label = if (expanded) stringResource(R.string.participants_show_fewer) else pluralStringResource(R.plurals.participants_more, hidden, hidden)
            val desc = if (expanded) label else pluralStringResource(R.plurals.participants_show_all, meeting.attendees.size, meeting.attendees.size)
            TextButton(onClick = { expanded = !expanded }, modifier = Modifier.semantics { contentDescription = desc }) { Text(label) }
        }
    }
}

/** The summary blocks; the synced action items win over the summary's own lines. */
private fun androidx.compose.foundation.lazy.LazyListScope.summary(meeting: PhoneMeeting, synced: List<Pair<String, Boolean>>) {
    val actions = synced.ifEmpty { meeting.summaryActionItems.map { it to null } }
    if (meeting.summaryTldr == null && meeting.summaryDecisions.isEmpty() && actions.isEmpty() && meeting.summaryOpenQuestions.isEmpty()) {
        item { Muted(stringResource(R.string.detail_no_summary)) }
        return
    }
    meeting.summaryTldr?.let { tldr -> item(key = "tldr") { Block(stringResource(R.string.detail_tldr)) { Text(tldr) } } }
    if (meeting.summaryDecisions.isNotEmpty()) item(key = "decisions") { Lines(stringResource(R.string.detail_decisions), meeting.summaryDecisions.map { it to null }) }
    if (actions.isNotEmpty()) item(key = "actions") { Lines(stringResource(R.string.detail_action_items), actions) }
    if (meeting.summaryOpenQuestions.isNotEmpty()) item(key = "questions") { Lines(stringResource(R.string.detail_open_questions), meeting.summaryOpenQuestions.map { it to null }) }
}

@Composable
private fun Block(title: String, content: @Composable () -> Unit) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        Text(title.uppercase(), style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.semantics { heading() })
        content()
    }
}

/** [done] null draws a bullet; true a check, false an empty circle. */
@Composable
private fun Lines(title: String, lines: List<Pair<String, Boolean?>>) = Block(title) {
    Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
        lines.forEach { (text, done) ->
            val label = if (done == true) stringResource(R.string.detail_item_done, text) else text
            Row(Modifier.semantics(mergeDescendants = true) { contentDescription = label }, horizontalArrangement = Arrangement.spacedBy(8.dp), verticalAlignment = Alignment.Top) {
                Box(Modifier.size(20.dp), contentAlignment = Alignment.Center) {
                    when (done) {
                        null -> Text("•", color = MaterialTheme.colorScheme.onSurfaceVariant)
                        true -> Icon(Icons.Filled.CheckCircle, null, tint = com.mpointer.whimbrel.mobile.theme.LocalThemePalette.current.success, modifier = Modifier.size(20.dp))
                        false -> Box(Modifier.size(16.dp).border(1.5.dp, MaterialTheme.colorScheme.onSurfaceVariant, CircleShape))
                    }
                }
                Text(text, Modifier.clearAndSetSemantics {})
            }
        }
    }
}
