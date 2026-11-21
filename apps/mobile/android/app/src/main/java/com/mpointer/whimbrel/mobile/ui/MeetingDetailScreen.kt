package com.mpointer.whimbrel.mobile.ui

import android.text.format.DateUtils
import androidx.compose.foundation.border
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.background
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
import androidx.compose.runtime.CompositionLocalProvider
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
import com.mpointer.whimbrel.mobile.design.LocalWStrings
import com.mpointer.whimbrel.mobile.design.WAuthor
import com.mpointer.whimbrel.mobile.design.WButton
import com.mpointer.whimbrel.mobile.design.WButtonKind
import com.mpointer.whimbrel.mobile.design.WFixed
import com.mpointer.whimbrel.mobile.design.LocalWAppearance
import com.mpointer.whimbrel.mobile.design.WNoteBlock
import com.mpointer.whimbrel.mobile.design.WNoteKind
import com.mpointer.whimbrel.mobile.design.WStateKind
import com.mpointer.whimbrel.mobile.design.WStateView
import com.mpointer.whimbrel.mobile.design.WTokens
import com.mpointer.whimbrel.mobile.design.WTranscriptLine
import com.mpointer.whimbrel.mobile.design.color
import com.mpointer.whimbrel.mobile.design.reading
import com.mpointer.whimbrel.mobile.design.style
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

    // Every summary line on this page is Whimbrel's, said once in the legend above them, so TalkBack is
    // not told "Whimbrel wrote" before each line.
    val base = LocalWStrings.current
    CompositionLocalProvider(LocalWStrings provides remember(base) { base.copy(whimbrelWrote = { it }) }) {
        Column(Modifier.fillMaxSize()) {
            if (onBack != null) {
                TopAppBar(title = {}, navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) } })
            } else {
                Box(Modifier.statusBarsPadding())
            }
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                LazyColumn(
                    Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxSize(),
                    contentPadding = androidx.compose.foundation.layout.PaddingValues(horizontal = WTokens.Space.lg, vertical = WTokens.Space.md),
                    verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
                ) {
                    val m = meeting
                    if (m == null) {
                        item {
                            if (loaded) WStateView(WStateKind.Empty, stringResource(R.string.detail_not_here))
                            else WStateView(WStateKind.Loading, stringResource(R.string.today_loading))
                        }
                        return@LazyColumn
                    }
                    item(key = "header") { Header(m, labels) }
                    if (m.attendees.isNotEmpty()) item(key = "participants") { Participants(m) }
                    item(key = "picker") {
                        val names = listOf(R.string.detail_summary, R.string.detail_transcript)
                        val section = stringResource(R.string.detail_section)
                        SingleChoiceSegmentedButtonRow(Modifier.fillMaxWidth().padding(vertical = WTokens.Space.sm).semantics { contentDescription = section }) {
                            names.forEachIndexed { i, name ->
                                SegmentedButton(selected = tab == i, onClick = { tab = i }, shape = SegmentedButtonDefaults.itemShape(i, names.size), label = { Text(stringResource(name)) })
                            }
                        }
                    }
                    if (tab == SUMMARY) {
                        summary(m, items.map { it.text to it.completed })
                    } else if (loaded && total == 0) {
                        item { WStateView(WStateKind.Empty, stringResource(R.string.detail_no_transcript)) }
                    } else {
                        items(segments, key = { it.id }) { segment ->
                            WTranscriptLine(transcriptTime(segment.startCs), segment.text, speaker = segment.speakerLabel.ifEmpty { null })
                        }
                        if (model.hasMoreTranscript) item(key = "more") {
                            WButton(stringResource(R.string.detail_load_more), { scope.launch { model.loadMoreTranscript(sync) } }, Modifier.fillMaxWidth(), kind = WButtonKind.Secondary)
                        }
                    }
                }
            }
        }
    }
}

/** Whimbrel's ink for the phone's current appearance. */
@Composable
private fun aiInk() = LocalWAppearance.current.let { WFixed.aiInk.color(it.dark, it.highContrast) }

@Composable
private fun Muted(text: String) = Text(text, style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)

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
        Text(meeting.title, style = WTokens.TypeScale.readingTitle.reading(), modifier = Modifier.semantics { heading() })
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

/** The summary, in Whimbrel's ink; the synced action items win over the summary's own lines. */
private fun androidx.compose.foundation.lazy.LazyListScope.summary(meeting: PhoneMeeting, synced: List<Pair<String, Boolean>>) {
    val actions: List<Pair<String, Boolean?>> = synced.ifEmpty { meeting.summaryActionItems.map { it to null } }
    if (meeting.summaryTldr == null && meeting.summaryDecisions.isEmpty() && actions.isEmpty() && meeting.summaryOpenQuestions.isEmpty()) {
        item { WStateView(WStateKind.Empty, stringResource(R.string.detail_no_summary)) }
        return
    }
    item(key = "legend") { WrittenByLegend() }
    meeting.summaryTldr?.let { tldr ->
        item(key = "tldr") { Section(stringResource(R.string.detail_tldr)) { WNoteBlock(WNoteKind.Paragraph, WAuthor.Whimbrel, tldr) } }
    }
    if (meeting.summaryDecisions.isNotEmpty()) item(key = "decisions") { Lines(stringResource(R.string.detail_decisions), meeting.summaryDecisions.map { it to null }) }
    if (actions.isNotEmpty()) item(key = "actions") { Lines(stringResource(R.string.detail_action_items), actions) }
    if (meeting.summaryOpenQuestions.isNotEmpty()) item(key = "questions") { Lines(stringResource(R.string.detail_open_questions), meeting.summaryOpenQuestions.map { it to null }) }
}

/** Says once, in the ink and rule the lines below use, who wrote them. */
@Composable
internal fun WrittenByLegend() {
    val ink = aiInk()
    // The rule is as tall as the words, at any text size.
    Row(Modifier.padding(top = WTokens.Space.sm).height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm), verticalAlignment = Alignment.CenterVertically) {
        Box(Modifier.width(2.dp).fillMaxHeight().background(ink))
        Text(stringResource(R.string.detail_written_by), style = WTokens.TypeScale.caption.style().copy(fontWeight = FontWeight.SemiBold), color = ink)
    }
}

@Composable
private fun Section(title: String, content: @Composable () -> Unit) {
    Column(Modifier.padding(top = WTokens.Space.sm), verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
        WNoteBlock(WNoteKind.Heading, WAuthor.You, title)
        content()
    }
}

/**
 * Lines Whimbrel wrote. [done] null draws a bullet. True or false draws a mark that shows whether the action
 * item is done; it cannot be changed here, so it is a line with a mark, not a control.
 */
@Composable
private fun Lines(title: String, lines: List<Pair<String, Boolean?>>) = Section(title) {
    val ink = aiInk()
    lines.forEach { (text, done) ->
        if (done == null) {
            WNoteBlock(WNoteKind.Bullet, WAuthor.Whimbrel, text)
        } else {
            val label = if (done) stringResource(R.string.detail_item_done, text) else text
            Row(
                Modifier.fillMaxWidth().height(IntrinsicSize.Min).padding(vertical = WTokens.Space.xs).semantics(mergeDescendants = true) { contentDescription = label },
                horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
            ) {
                Box(Modifier.width(2.dp).fillMaxHeight().background(ink))
                Box(Modifier.padding(top = 4.dp).size(20.dp), contentAlignment = Alignment.Center) {
                    if (done) Icon(Icons.Filled.CheckCircle, null, tint = com.mpointer.whimbrel.mobile.theme.LocalThemePalette.current.success, modifier = Modifier.size(20.dp))
                    else Box(Modifier.size(16.dp).border(1.5.dp, ink, CircleShape))
                }
                Text(text, Modifier.weight(1f).clearAndSetSemantics {}, style = WTokens.TypeScale.reading.reading(), color = ink)
            }
        }
    }
}
