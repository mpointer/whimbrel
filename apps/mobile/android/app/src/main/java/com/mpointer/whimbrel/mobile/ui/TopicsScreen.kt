package com.mpointer.whimbrel.mobile.ui

import androidx.activity.compose.BackHandler
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.DateRange
import androidx.compose.material.icons.filled.Email
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Person
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.design.WCard
import com.mpointer.whimbrel.mobile.design.WStateKind
import com.mpointer.whimbrel.mobile.design.WStateView
import com.mpointer.whimbrel.mobile.design.WTokens
import com.mpointer.whimbrel.mobile.design.reading
import com.mpointer.whimbrel.mobile.design.style
import com.mpointer.whimbrel.mobile.read.PhoneMeetingRef
import com.mpointer.whimbrel.mobile.read.PhoneTopicSnapshot
import com.mpointer.whimbrel.mobile.read.ResourceTopicStrings
import com.mpointer.whimbrel.mobile.read.TopicPageModel
import com.mpointer.whimbrel.mobile.read.TopicStrings
import com.mpointer.whimbrel.mobile.read.TopicText
import com.mpointer.whimbrel.mobile.read.TopicsModel
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette
import java.time.LocalDate
import kotlinx.coroutines.delay
import kotlinx.coroutines.launch

private val cardShape = RoundedCornerShape(WTokens.Radius.card)

/**
 * Topics: the topic pages the computer built, each a card with its name, its lead and the next meeting's time;
 * a tapped one opens its page in place. Read-only. Ported from `TopicsView.swift`.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TopicsScreen(sync: SyncModel, model: TopicsModel, openKey: String?, onOpenTopic: (String?) -> Unit, onOpenMeeting: (Long) -> Unit) {
    if (openKey != null) {
        BackHandler { onOpenTopic(null) }
        TopicPageScreen(openKey, sync, onBack = { onOpenTopic(null) }, onOpenMeeting = onOpenMeeting)
        return
    }
    val scope = rememberCoroutineScope() // Main
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(revision) { model.reload(sync) }
    val topics by model.topics.collectAsState()
    val loaded by model.loaded.collectAsState()
    val context = LocalContext.current
    val strings = remember(context) { ResourceTopicStrings(context) }
    var refreshing by remember { mutableStateOf(false) }

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
        modifier = Modifier.fillMaxSize().statusBarsPadding(),
    ) {
        Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
            LazyColumn(
                Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxSize(),
                contentPadding = PaddingValues(horizontal = WTokens.Space.lg, vertical = WTokens.Space.md),
                verticalArrangement = Arrangement.spacedBy(WTokens.Space.md),
            ) {
                item(key = "title") {
                    Text(stringResource(R.string.tab_topics), Modifier.padding(top = WTokens.Space.xl).semantics { heading() }, style = WTokens.TypeScale.pageTitle.style())
                }
                items(topics, key = { it.topicKey }) { topic -> TopicRow(topic, strings) { onOpenTopic(topic.topicKey) } }
                if (topics.isEmpty()) item {
                    WStateView(if (loaded) WStateKind.Empty else WStateKind.Loading, stringResource(if (loaded) R.string.topics_empty else R.string.today_loading))
                }
            }
        }
    }
}

/** One topic in the list: its name, its lead and when the next meeting is. The whole card is the button. */
@Composable
private fun TopicRow(topic: PhoneTopicSnapshot, strings: TopicStrings, onClick: () -> Unit) {
    val now = System.currentTimeMillis()
    val scheme = MaterialTheme.colorScheme
    WCard(Modifier.clip(cardShape).clickable(role = Role.Button, onClick = onClick), raised = true) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
            Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
                Text(topic.name, style = WTokens.TypeScale.subtitle.style())
                topic.lead?.let {
                    Text(TopicText.lead(it, LocalDate.now(), strings), style = WTokens.TypeScale.secondary.style(), color = scheme.onSurfaceVariant, maxLines = 3, overflow = TextOverflow.Ellipsis)
                }
                topic.upcomingMeeting(now)?.let { next ->
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
                        Icon(Icons.Filled.DateRange, null, Modifier.size(textIconSize(16)), tint = scheme.primary)
                        Text(strings.dayTime(next.startUnixMs), style = WTokens.TypeScale.time.style(), color = scheme.primary)
                    }
                }
            }
            Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = scheme.onSurfaceVariant)
        }
    }
}

/**
 * A topic page as the computer built it, read-only, in the computer's order: the lead, your role, the next
 * meeting, what you owe, what is owed to you, what was done recently and recent items. A step or item whose
 * meeting is on this phone opens it. Ported from `TopicPageView.swift`.
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun TopicPageScreen(key: String, sync: SyncModel, onBack: () -> Unit, onOpenMeeting: (Long) -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val model = remember(key) { TopicPageModel(key) }
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(model, revision) { model.reload(sync) }
    val snapshot by model.snapshot.collectAsState()
    val meetingIds by model.meetingIds.collectAsState()
    val loaded by model.loaded.collectAsState()
    val context = LocalContext.current
    val strings = remember(context) { ResourceTopicStrings(context) }
    var refreshing by remember { mutableStateOf(false) }
    // The footer says how long ago the page was built, so it moves on while the page is open.
    val now by produceState(System.currentTimeMillis()) {
        while (true) {
            delay(60_000)
            value = System.currentTimeMillis()
        }
    }

    Column(Modifier.fillMaxSize()) {
        TopAppBar(title = {}, navigationIcon = { IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) } })
        PullToRefreshBox(
            isRefreshing = refreshing,
            onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
            modifier = Modifier.fillMaxSize(),
        ) {
            Box(Modifier.fillMaxSize(), contentAlignment = Alignment.TopCenter) {
                LazyColumn(
                    Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxSize(),
                    contentPadding = PaddingValues(horizontal = WTokens.Space.lg, vertical = WTokens.Space.md),
                    verticalArrangement = Arrangement.spacedBy(WTokens.Space.lg),
                ) {
                    val snap = snapshot
                    if (snap == null) {
                        item {
                            if (loaded) WStateView(WStateKind.Empty, stringResource(R.string.topic_not_here))
                            else WStateView(WStateKind.Loading, stringResource(R.string.today_loading))
                        }
                        return@LazyColumn
                    }
                    val today = LocalDate.now()
                    val open: (PhoneMeetingRef?) -> (() -> Unit)? = { ref -> ref?.let(meetingIds::get)?.let { id -> { onOpenMeeting(id) } } }
                    item(key = "header") {
                        Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
                            Text(snap.name, Modifier.semantics { heading() }, style = WTokens.TypeScale.readingTitle.reading())
                            snap.lead?.let {
                                Text(TopicText.lead(it, today, strings), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                        }
                    }
                    snap.role?.takeIf { it.text.isNotEmpty() }?.let { role ->
                        item(key = "role") { Section(R.string.topic_your_role) { Text(role.text, style = WTokens.TypeScale.body.style()) } }
                    }
                    snap.upcomingMeeting(now)?.let { next ->
                        item(key = "next") { Section(R.string.topic_next_meeting) { NextMeeting(next, strings) } }
                    }
                    item(key = "owe") { Section(R.string.topic_you_owe) { Steps(snap.youOwe, R.string.topic_nothing_you_owe, false, today, strings, open) } }
                    item(key = "owed") { Section(R.string.topic_owed_to_you) { Steps(snap.owedToYou, R.string.topic_nothing_owed, false, today, strings, open) } }
                    if (snap.done.isNotEmpty()) {
                        item(key = "done") { Section(R.string.topic_done_recently) { Steps(snap.done, null, true, today, strings, open) } }
                    }
                    item(key = "recent") { Section(R.string.topic_recent) { Recent(snap, strings, open) } }
                    if (snap.truncated) item(key = "truncated") {
                        Text(stringResource(R.string.topic_truncated), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                    }
                    item(key = "footer") {
                        Text(
                            TopicText.footer(snap, now, strings), Modifier.fillMaxWidth(), textAlign = TextAlign.Center,
                            style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant,
                        )
                    }
                }
            }
        }
    }
}

/** A titled card; the title is a heading, so TalkBack can move section by section. */
@Composable
private fun Section(title: Int, content: @Composable () -> Unit) {
    WCard(raised = true) {
        Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
            Text(
                stringResource(title).uppercase(), Modifier.semantics { heading() },
                style = WTokens.TypeScale.caption.style().copy(fontWeight = FontWeight.Bold, letterSpacing = 0.6.sp),
                color = MaterialTheme.colorScheme.onSurfaceVariant,
            )
            content()
        }
    }
}

@Composable
private fun NextMeeting(next: PhoneTopicSnapshot.NextMeeting, strings: TopicStrings) {
    Column(Modifier.semantics(mergeDescendants = true) {}, verticalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
        Text(next.title, style = WTokens.TypeScale.subtitle.style())
        Text(strings.dayTime(next.startUnixMs), style = WTokens.TypeScale.time.style(), color = MaterialTheme.colorScheme.primary)
        TopicText.attendees(next, strings).takeIf { it.isNotEmpty() }?.let {
            Text(it, style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}

@Composable
private fun Steps(
    list: List<PhoneTopicSnapshot.Step>, empty: Int?, done: Boolean, today: LocalDate, strings: TopicStrings,
    open: (PhoneMeetingRef?) -> (() -> Unit)?,
) {
    if (list.isEmpty()) {
        if (empty != null) Text(stringResource(empty), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.md)) {
        list.forEach { step -> Linked(open(step.meeting)) { StepRow(step, done, today, strings) } }
    }
}

/**
 * A step and where it came from. A done step has a check as well as the strike, and says so to a screen reader.
 * The details wrap onto more lines when the text is large.
 */
@OptIn(ExperimentalLayoutApi::class)
@Composable
private fun StepRow(step: PhoneTopicSnapshot.Step, done: Boolean, today: LocalDate, strings: TopicStrings) {
    val scheme = MaterialTheme.colorScheme
    val spoken = if (done) stringResource(R.string.detail_item_done, step.text) else step.text
    Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
        if (done) Icon(Icons.Filled.CheckCircle, null, Modifier.padding(top = 2.dp), tint = LocalThemePalette.current.success)
        Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
            Text(
                step.text, Modifier.semantics { contentDescription = spoken },
                style = WTokens.TypeScale.body.style().copy(textDecoration = if (done) TextDecoration.LineThrough else null),
            )
            val caption = WTokens.TypeScale.caption.style()
            FlowRow(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm), verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
                step.sourceTitle?.takeIf { it.isNotEmpty() }?.let { title ->
                    Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.xs)) {
                        Icon(kindIcon(step.sourceKind), null, Modifier.size(textIconSize(14)), tint = scheme.onSurfaceVariant)
                        Text(title, style = caption, color = scheme.onSurfaceVariant, maxLines = 2, overflow = TextOverflow.Ellipsis)
                    }
                }
                step.askedAtUnixMs?.let { Text(strings.asked(strings.monthDay(it)), style = caption, color = scheme.onSurfaceVariant) }
                step.dueDate?.let { due ->
                    val late = TopicText.isOverdue(due, today) && !done
                    Text(
                        TopicText.dueText(due, today, strings),
                        style = caption.copy(fontWeight = if (late) FontWeight.SemiBold else FontWeight.Normal),
                        color = if (late) scheme.error else scheme.onSurfaceVariant,
                    )
                }
            }
        }
    }
}

@Composable
private fun Recent(snap: PhoneTopicSnapshot, strings: TopicStrings, open: (PhoneMeetingRef?) -> (() -> Unit)?) {
    val scheme = MaterialTheme.colorScheme
    if (snap.recent.isEmpty()) {
        Text(stringResource(R.string.topic_nothing_yet), style = WTokens.TypeScale.secondary.style(), color = scheme.onSurfaceVariant)
        return
    }
    Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.md)) {
        snap.recent.forEach { item ->
            Linked(open(item.meeting)) {
                Row(horizontalArrangement = Arrangement.spacedBy(WTokens.Space.md)) {
                    Icon(kindIcon(item.kind), null, Modifier.padding(top = 2.dp).size(textIconSize(20)), tint = scheme.onSurfaceVariant)
                    Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.xxs)) {
                        Text(item.title.ifEmpty { item.kind.replaceFirstChar(Char::uppercase) }, style = WTokens.TypeScale.body.style())
                        Text(
                            (listOfNotNull(item.sender) + strings.monthDay(item.tsUnixMs)).joinToString(" · "),
                            style = WTokens.TypeScale.secondary.style(), color = scheme.onSurfaceVariant,
                        )
                        if (item.kind == "email") {
                            Text(stringResource(R.string.topic_open_on_computer), style = WTokens.TypeScale.caption.style(), color = scheme.onSurfaceVariant)
                        }
                    }
                }
            }
        }
        val older = (snap.recentTotal - snap.recent.size).toInt()
        if (older > 0) {
            Text(pluralStringResource(R.plurals.topic_older_items, older, older), style = WTokens.TypeScale.caption.style(), color = scheme.onSurfaceVariant)
        }
    }
}

/** A row that opens its meeting when this phone has it (a chevron, a full-height target, "Opens the source"); otherwise the row as it is. */
@Composable
private fun Linked(onOpen: (() -> Unit)?, content: @Composable () -> Unit) {
    if (onOpen == null) {
        Box(Modifier.fillMaxWidth().semantics(mergeDescendants = true) {}) { content() }
        return
    }
    Row(
        // No rounded clip here: the row's text reaches its corners, and a clip cut into the first letter.
        Modifier.fillMaxWidth()
            .clickable(onClickLabel = stringResource(R.string.action_open_source), role = Role.Button, onClick = onOpen)
            .heightIn(min = WTokens.Size.target),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
    ) {
        Box(Modifier.weight(1f)) { content() }
        Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

private fun kindIcon(kind: String): ImageVector = when (kind) {
    "meeting" -> Icons.Filled.Person
    "email" -> Icons.Filled.Email
    else -> Icons.Filled.Info
}
