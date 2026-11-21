package com.mpointer.whimbrel.mobile.ui

import android.text.format.DateUtils
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.ime
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.layout.widthIn
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.foundation.lazy.rememberLazyListState
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.KeyboardActions
import androidx.compose.foundation.text.KeyboardOptions
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.automirrored.filled.Send
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Info
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.FilledIconButton
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.OutlinedTextField
import androidx.compose.material3.Text
import androidx.compose.material3.pulltorefresh.PullToRefreshBox
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableIntStateOf
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.layout.boundsInWindow
import androidx.compose.ui.layout.onGloballyPositioned
import androidx.compose.ui.platform.LocalClipboardManager
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.platform.LocalView
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.AnnotatedString
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.input.ImeAction
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.ask.AskModel
import com.mpointer.whimbrel.mobile.ask.AskRoute
import com.mpointer.whimbrel.mobile.ask.AskTurn
import com.mpointer.whimbrel.mobile.ask.CitationCard
import com.mpointer.whimbrel.mobile.ask.CitationTarget
import com.mpointer.whimbrel.mobile.ask.CitationText
import com.mpointer.whimbrel.mobile.design.LocalWStrings
import com.mpointer.whimbrel.mobile.design.WAuthor
import com.mpointer.whimbrel.mobile.design.WCard
import com.mpointer.whimbrel.mobile.design.WNoteBlock
import com.mpointer.whimbrel.mobile.design.WNoteKind
import com.mpointer.whimbrel.mobile.design.WTokens
import com.mpointer.whimbrel.mobile.design.reading
import com.mpointer.whimbrel.mobile.design.style
import com.mpointer.whimbrel.mobile.read.PhoneCaptureItem
import com.mpointer.whimbrel.mobile.read.PhoneCaptureStore
import com.mpointer.whimbrel.mobile.read.PhoneRoutineRun
import com.mpointer.whimbrel.mobile.read.ReadStrings
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

private val cardShape = RoundedCornerShape(WTokens.Radius.card)

/**
 * Ask about your meetings, captures and routine runs, answered by the user's own hosted key. Ported from
 * `AskView.swift`, without dictation and read-aloud (not on Android yet).
 */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun AskScreen(sync: SyncModel, model: AskModel, labels: StateFlow<DeviceLabels>, onOpenMeeting: (Long) -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val turns by model.turns.collectAsState()
    val syncing by sync.isSyncing.collectAsState()
    val deviceLabels by labels.collectAsState()
    val context = LocalContext.current
    val strings = remember(context) { ResourceReadStrings(context) }
    // A key can arrive one sync round after its toggle: look again whenever a round ends. Null until first read.
    var route by remember { mutableStateOf<AskRoute?>(null) }
    LaunchedEffect(syncing) { if (!syncing || route == null) route = withContext(Dispatchers.IO) { model.route } }
    var draft by rememberSaveable { mutableStateOf("") }
    var captureId by rememberSaveable { mutableStateOf<Long?>(null) }
    var runId by rememberSaveable { mutableStateOf<Long?>(null) }
    var refreshing by remember { mutableStateOf(false) }
    val ready = route is AskRoute.Ready

    // The keyboard covers the bottom of the window; the tab bar under this screen already takes part of that,
    // so only the rest is padded.
    val density = LocalDensity.current
    val windowHeight = LocalView.current.rootView.height
    var bottomInWindow by remember { mutableIntStateOf(0) }
    val imePx = WindowInsets.ime.getBottom(density)
    val overlap = if (bottomInWindow == 0) 0 else (imePx - (windowHeight - bottomInWindow)).coerceAtLeast(0)

    // The answer block marks Whimbrel's words with its ink and rule, and the question above it is read as
    // "You asked", so TalkBack is not also given an English-only prefix.
    val base = LocalWStrings.current
    CompositionLocalProvider(LocalWStrings provides remember(base) { base.copy(whimbrelWrote = { it }) }) {
        Box(Modifier.fillMaxSize().onGloballyPositioned { bottomInWindow = it.boundsInWindow().bottom.toInt() }) {
            Column(Modifier.fillMaxSize().statusBarsPadding().padding(bottom = with(density) { overlap.toDp() })) {
                Box(Modifier.weight(1f).fillMaxWidth(), contentAlignment = Alignment.TopCenter) {
                    if (ready) {
                        val list = rememberLazyListState()
                        // Follow the answer as it grows, and a new question as it is added.
                        LaunchedEffect(turns.size, turns.lastOrNull()?.answer?.length, turns.lastOrNull()?.state) {
                            if (turns.isNotEmpty()) list.scrollToItem(turns.size, Int.MAX_VALUE / 2)
                        }
                        LazyColumn(
                            Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxSize(), state = list,
                            contentPadding = PaddingValues(horizontal = WTokens.Space.lg, vertical = WTokens.Space.md),
                            verticalArrangement = Arrangement.spacedBy(WTokens.Space.xl),
                        ) {
                            item(key = "title") { Title() }
                            if (turns.isEmpty()) item(key = "intro") {
                                Text(stringResource(R.string.ask_intro), style = WTokens.TypeScale.body.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                            }
                            items(turns, key = { it.id }) { turn ->
                                TurnView(turn, deviceLabels, strings, onOpenMeeting, openCapture = { captureId = it }, openRun = { runId = it })
                            }
                        }
                    } else {
                        // Pull to refresh runs a sync round, which also pulls the settings and keys.
                        PullToRefreshBox(
                            isRefreshing = refreshing,
                            onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
                            modifier = Modifier.fillMaxSize(),
                        ) {
                            Column(
                                Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxSize().verticalScroll(rememberScrollState())
                                    .padding(horizontal = WTokens.Space.lg, vertical = WTokens.Space.md),
                                verticalArrangement = Arrangement.spacedBy(WTokens.Space.md),
                            ) {
                                Title()
                                when (route) {
                                    AskRoute.NoHostedKey -> Message(R.string.ask_no_hosted_key)
                                    AskRoute.LocalOnly -> Message(R.string.ask_local_only)
                                    else -> {}
                                }
                            }
                        }
                    }
                }
                if (ready) {
                    Composer(
                        draft = draft, onDraft = { draft = it }, busy = model.isBusyNow(turns),
                        onSend = {
                            if (draft.isNotBlank() && !model.isBusy) {
                                model.send(draft)
                                draft = ""
                            }
                        },
                        onStop = model::stop,
                    )
                }
            }
        }
    }
    captureId?.let { id -> CaptureSheet(id, sync) { captureId = null } }
    runId?.let { id -> RunSheet(id, sync, deviceLabels, strings) { runId = null } }
}

/** True while the newest turn is still streaming; read from the collected [turns] so the composer recomposes with it. */
private fun AskModel.isBusyNow(turns: List<AskTurn>): Boolean = turns.lastOrNull()?.state == AskTurn.State.Streaming

@Composable
private fun Title() {
    Text(stringResource(R.string.tab_ask), Modifier.padding(top = WTokens.Space.xl).semantics { heading() }, style = WTokens.TypeScale.pageTitle.style())
}

@Composable
private fun Message(text: Int) {
    WCard(raised = true) { Text(stringResource(text), style = WTokens.TypeScale.body.style()) }
}

@Composable
private fun Composer(draft: String, onDraft: (String) -> Unit, busy: Boolean, onSend: () -> Unit, onStop: () -> Unit) {
    val question = stringResource(R.string.ask_question_field)
    Box(Modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Row(
            Modifier.widthIn(max = WTokens.Size.contentMax).fillMaxWidth().padding(horizontal = WTokens.Space.lg, vertical = WTokens.Space.sm),
            verticalAlignment = Alignment.Bottom, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
        ) {
            OutlinedTextField(
                value = draft, onValueChange = onDraft,
                modifier = Modifier.weight(1f).heightIn(min = WTokens.Size.target).semantics { contentDescription = question },
                placeholder = { Text(stringResource(R.string.ask_placeholder)) },
                textStyle = WTokens.TypeScale.body.style(), maxLines = 5, shape = RoundedCornerShape(22.dp),
                keyboardOptions = KeyboardOptions(imeAction = ImeAction.Send), keyboardActions = KeyboardActions(onSend = { onSend() }),
            )
            if (busy) {
                FilledIconButton(onClick = onStop, Modifier.size(WTokens.Size.target)) { Icon(StopIcon, stringResource(R.string.ask_stop)) }
            } else {
                FilledIconButton(onClick = onSend, Modifier.size(WTokens.Size.target), enabled = draft.isNotBlank()) {
                    Icon(Icons.AutoMirrored.Filled.Send, stringResource(R.string.ask_send))
                }
            }
        }
    }
}

/**
 * One question and its answer. The question is in your ink and is the turn's heading; the answer is in
 * Whimbrel's ink with its rule, as on the note page.
 */
@Composable
private fun TurnView(
    turn: AskTurn, labels: DeviceLabels, strings: ReadStrings,
    onOpenMeeting: (Long) -> Unit, openCapture: (Long) -> Unit, openRun: (Long) -> Unit,
) {
    val scheme = MaterialTheme.colorScheme
    val asked = stringResource(R.string.ask_you_asked, turn.question)
    Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
        Text(turn.question, Modifier.fillMaxWidth().semantics { heading(); contentDescription = asked }, style = WTokens.TypeScale.subtitle.style())
        if (turn.answer.isNotEmpty()) SelectionContainer { WNoteBlock(WNoteKind.Paragraph, WAuthor.Whimbrel, turn.answer) }
        when (turn.state) {
            AskTurn.State.Streaming -> if (turn.answer.isEmpty()) {
                val thinking = stringResource(R.string.ask_thinking)
                Box(Modifier.heightIn(min = WTokens.Size.target).semantics { contentDescription = thinking }, contentAlignment = Alignment.CenterStart) {
                    CircularProgressIndicator(Modifier.size(24.dp), strokeWidth = 2.dp)
                }
            }
            AskTurn.State.Done -> {}
            AskTurn.State.Stopped -> Note(R.string.ask_stopped, isError = false)
            AskTurn.State.SpendCap -> Note(R.string.ask_spend_cap, isError = false)
            AskTurn.State.Failed -> Note(R.string.ask_failed, isError = true)
        }
        if (turn.citations.isNotEmpty()) {
            Column(verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                turn.citations.forEach { card -> CitationChip(card, labels, strings, onOpenMeeting, openCapture, openRun) }
            }
        }
        // Which model answered and whose key paid, on its own line so large text does not squeeze it, then Copy.
        if (turn.state != AskTurn.State.Streaming && turn.answer.isNotEmpty()) {
            val clipboard = LocalClipboardManager.current
            val copyAnswer = stringResource(R.string.ask_copy_answer)
            Column {
                Text(stringResource(R.string.ask_model_line, turn.model), style = WTokens.TypeScale.caption.style(), color = scheme.onSurfaceVariant)
                Box(
                    Modifier.clip(RoundedCornerShape(WTokens.Radius.control))
                        .clickable(role = Role.Button) { clipboard.setText(AnnotatedString(turn.answer)) }
                        .heightIn(min = WTokens.Size.target).widthIn(min = WTokens.Size.target).semantics { contentDescription = copyAnswer },
                    contentAlignment = Alignment.CenterStart,
                ) { Text(stringResource(R.string.ask_copy), style = WTokens.TypeScale.time.style(), color = scheme.primary) }
            }
        }
    }
}

/** How a run ended, as an icon and words: never by color alone. */
@Composable
private fun Note(text: Int, isError: Boolean) {
    val color = if (isError) MaterialTheme.colorScheme.error else MaterialTheme.colorScheme.onSurfaceVariant
    Row(Modifier.semantics(mergeDescendants = true) {}, verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
        Icon(if (isError) Icons.Filled.Warning else Icons.Filled.Info, null, Modifier.size(textIconSize(18)), tint = color)
        Text(stringResource(text), style = WTokens.TypeScale.secondary.style(), color = color)
    }
}

/**
 * One source under an answer: a card that opens its meeting, its routine run or its captured text. One that does
 * not open (not on this phone, or a kind with no screen here) is plain text on the same card, with no chevron.
 */
@Composable
private fun CitationChip(
    card: CitationCard, labels: DeviceLabels, strings: ReadStrings,
    onOpenMeeting: (Long) -> Unit, openCapture: (Long) -> Unit, openRun: (Long) -> Unit,
) {
    val context = LocalContext.current
    val now = System.currentTimeMillis()
    val text = CitationText.label(card, card.deviceId?.let { strings.device(labels.label(it)) }, stringResource(R.string.citation_not_here)) { ms ->
        val flags = if (CitationText.isRecent(ms, now)) DateUtils.FORMAT_SHOW_WEEKDAY or DateUtils.FORMAT_ABBREV_WEEKDAY
        else DateUtils.FORMAT_SHOW_DATE or DateUtils.FORMAT_ABBREV_MONTH or DateUtils.FORMAT_NO_YEAR
        DateUtils.formatDateTime(context, ms, flags)
    }
    val target = CitationTarget.of(card)
    val open: (() -> Unit)? = when (target) {
        is CitationTarget.Meeting -> ({ onOpenMeeting(target.id) })
        is CitationTarget.Capture -> ({ openCapture(target.id) })
        is CitationTarget.RoutineRun -> ({ openRun(target.id) })
        CitationTarget.None -> null
    }
    val label = stringResource(if (target is CitationTarget.Capture) R.string.action_show_capture else R.string.action_open_source)
    val scheme = MaterialTheme.colorScheme
    WCard(Modifier.clip(cardShape).then(if (open != null) Modifier.clickable(onClickLabel = label, role = Role.Button, onClick = open) else Modifier)) {
        Row(Modifier.heightIn(min = WTokens.Size.target - WTokens.Space.md * 2), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
            Text(text, Modifier.weight(1f), style = WTokens.TypeScale.secondary.style(), color = if (card.found) scheme.onSurface else scheme.onSurfaceVariant)
            if (open != null) Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = scheme.onSurfaceVariant)
        }
    }
}

/** A capture's text, in a sheet (the replica has no other screen for one). */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun CaptureSheet(id: Long, sync: SyncModel, onDismiss: () -> Unit) {
    var item by remember(id) { mutableStateOf<PhoneCaptureItem?>(null) }
    var loaded by remember(id) { mutableStateOf(false) }
    LaunchedEffect(id) {
        item = sync.read { (it as? PhoneCaptureStore)?.captureItem(id) }
        loaded = true
    }
    val scheme = MaterialTheme.colorScheme
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl),
            verticalArrangement = Arrangement.spacedBy(WTokens.Space.md),
        ) {
            Text(stringResource(R.string.capture_title).uppercase(), Modifier.semantics { heading() }, style = WTokens.TypeScale.caption.style().copy(fontWeight = FontWeight.Bold), color = scheme.onSurfaceVariant)
            val shown = item
            if (shown != null) {
                Text(shown.title, style = WTokens.TypeScale.title.reading())
                shown.url?.takeIf { it.isNotEmpty() }?.let { Text(it, style = WTokens.TypeScale.secondary.style(), color = scheme.onSurfaceVariant) }
                SelectionContainer { Text(shown.text, style = WTokens.TypeScale.reading.reading()) }
            } else {
                Text(stringResource(if (loaded) R.string.capture_not_here else R.string.today_loading), style = WTokens.TypeScale.body.style(), color = scheme.onSurfaceVariant)
            }
        }
    }
}

/** A cited routine run, found by id among the newest runs (the replica has no getter), with its whole summary. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun RunSheet(id: Long, sync: SyncModel, labels: DeviceLabels, strings: ReadStrings, onDismiss: () -> Unit) {
    var run by remember(id) { mutableStateOf<PhoneRoutineRun?>(null) }
    var loaded by remember(id) { mutableStateOf(false) }
    LaunchedEffect(id) {
        run = sync.read { store -> store.routineRuns(1000).firstOrNull { it.id == id } }
        loaded = true
    }
    val scheme = MaterialTheme.colorScheme
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl),
            verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
        ) {
            val shown = run
            if (shown == null) {
                Text(stringResource(if (loaded) R.string.run_not_here else R.string.today_loading), style = WTokens.TypeScale.body.style(), color = scheme.onSurfaceVariant)
                return@Column
            }
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                if (shown.succeeded) Icon(Icons.Filled.CheckCircle, stringResource(R.string.routines_succeeded), tint = LocalThemePalette.current.success)
                else Icon(Icons.Filled.Warning, null, tint = scheme.error)
                Text(shown.routineName, Modifier.semantics { heading() }, style = WTokens.TypeScale.title.style())
            }
            if (!shown.succeeded) Text(stringResource(R.string.routines_failed), style = WTokens.TypeScale.caption.style().copy(fontWeight = FontWeight.SemiBold), color = scheme.error)
            SelectionContainer { Text(shown.outputSummary, style = WTokens.TypeScale.reading.reading()) }
            shown.originDeviceId?.let { Text(strings.device(labels.label(it)), style = WTokens.TypeScale.caption.style(), color = scheme.onSurfaceVariant) }
        }
    }
}
