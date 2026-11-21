package com.mpointer.whimbrel.mobile.design

import androidx.compose.foundation.BorderStroke
import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.ColumnScope
import androidx.compose.foundation.layout.ExperimentalLayoutApi
import androidx.compose.foundation.layout.FlowRow
import androidx.compose.foundation.layout.IntrinsicSize
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.RowScope
import androidx.compose.foundation.layout.fillMaxHeight
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.CircularProgressIndicator
import androidx.compose.material3.LocalContentColor
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.Offset
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.PathEffect
import androidx.compose.ui.graphics.drawscope.Stroke
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.clearAndSetSemantics
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.liveRegion
import androidx.compose.ui.semantics.LiveRegionMode
import androidx.compose.ui.semantics.selected
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.text.style.TextDecoration
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette

private val S = WTokens.Space
private val T = WTokens.TypeScale
private val cardShape = RoundedCornerShape(WTokens.Radius.card)
private val controlShape = RoundedCornerShape(WTokens.Radius.control)

@Composable private fun fixed(c: WFixedColor): Color = LocalWAppearance.current.let { c.color(it.dark, it.highContrast) }

private fun WTokens.TypeRole.tabular() = style().copy(fontFeatureSettings = "tnum")

// ---- Surfaces --------------------------------------------------------------------------------------------------

/** A card on the page. In dark mode and high contrast a border stands in for the shadow. */
@Composable
fun WCard(
    modifier: Modifier = Modifier,
    raised: Boolean = false,
    tint: Color? = null,
    border: Color? = null,
    content: @Composable ColumnScope.() -> Unit,
) {
    val appearance = LocalWAppearance.current
    val flat = appearance.dark || appearance.highContrast
    val level = if (raised) WTokens.Elevation.level2 else WTokens.Elevation.level1
    Surface(
        modifier = modifier.fillMaxWidth().then(if (flat) Modifier else Modifier.shadow(level.blur / 2, cardShape, clip = false)),
        shape = cardShape,
        color = tint ?: MaterialTheme.colorScheme.surfaceVariant,
        contentColor = MaterialTheme.colorScheme.onSurface,
        border = (border ?: if (flat) MaterialTheme.colorScheme.outline else null)?.let { BorderStroke(if (appearance.highContrast) 2.dp else 1.dp, it) },
    ) {
        Column(Modifier.heightIn(min = WTokens.Size.target).padding(if (raised) S.lg else S.md), verticalArrangement = Arrangement.spacedBy(S.xs), content = content)
    }
}

// ---- Buttons ---------------------------------------------------------------------------------------------------

enum class WButtonKind { Primary, Secondary, Quiet, Destructive, Recording }

@Composable
fun WButton(text: String, onClick: () -> Unit, modifier: Modifier = Modifier, kind: WButtonKind = WButtonKind.Primary, enabled: Boolean = true) {
    val m = modifier.heightIn(min = WTokens.Size.target)
    val label: @Composable RowScope.() -> Unit = { Text(text, style = T.bodyStrong.style(), textAlign = TextAlign.Center) }
    when (kind) {
        WButtonKind.Primary -> Button(onClick, m, enabled, content = label)
        WButtonKind.Recording -> Button(
            onClick, m, enabled,
            colors = ButtonDefaults.buttonColors(containerColor = fixed(WFixed.recording), contentColor = fixed(WFixed.onRecording)), content = label,
        )
        WButtonKind.Secondary -> OutlinedButton(onClick, m, enabled, colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.onSurface), content = label)
        WButtonKind.Quiet -> TextButton(onClick, m, enabled, content = label)
        WButtonKind.Destructive -> TextButton(onClick, m, enabled, colors = ButtonDefaults.textButtonColors(contentColor = MaterialTheme.colorScheme.error), content = label)
    }
}

// ---- Chips -----------------------------------------------------------------------------------------------------

/** Initials avatar and a name. */
@Composable
fun WSpeakerChip(name: String, modifier: Modifier = Modifier) {
    Row(
        modifier.border(1.dp, MaterialTheme.colorScheme.outline, controlShape).padding(start = S.xs, top = S.xs, bottom = S.xs, end = S.sm),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.xs),
    ) {
        Box(Modifier.size(22.dp).background(MaterialTheme.colorScheme.outline, CircleShape).clearAndSetSemantics {}, contentAlignment = Alignment.Center) {
            Text(speakerInitials(name), style = T.caption.style().copy(fontWeight = FontWeight.Bold, fontSize = T.minimumReadable))
        }
        Text(name, style = T.secondary.style().copy(fontWeight = FontWeight.Medium))
    }
}

/** A selectable filter (People, Apps, Dates). Selected is shown by a check mark as well as color. */
@Composable
fun WFilterChip(label: String, selected: Boolean, onClick: () -> Unit, modifier: Modifier = Modifier) {
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier.heightIn(min = WTokens.Size.target).clip(controlShape)
            .background(if (selected) scheme.primaryContainer else scheme.surfaceVariant)
            .border(1.dp, if (selected) scheme.primary else scheme.outline, controlShape)
            .clickable(role = Role.Checkbox, onClick = onClick)
            .semantics { this.selected = selected }
            .padding(horizontal = S.md),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.xs),
    ) {
        if (selected) Text("✓", Modifier.clearAndSetSemantics {}, style = T.secondary.style().copy(fontWeight = FontWeight.Bold))
        Text(label, style = T.secondary.style().copy(fontWeight = FontWeight.Medium), color = scheme.onSurface)
    }
}

/** The search field as a button: tapping it opens search. Typing happens on the search screen. */
@Composable
fun WSearchBar(onClick: () -> Unit, modifier: Modifier = Modifier) {
    val strings = LocalWStrings.current
    Row(
        modifier.fillMaxWidth().heightIn(min = 56.dp).clip(RoundedCornerShape(WTokens.Radius.surface))
            .background(MaterialTheme.colorScheme.surfaceVariant)
            .border(1.dp, MaterialTheme.colorScheme.outline, RoundedCornerShape(WTokens.Radius.surface))
            .clickable(role = Role.Button, onClick = onClick).padding(horizontal = S.lg),
        verticalAlignment = Alignment.CenterVertically,
    ) {
        Text(strings.searchPlaceholder, style = T.body.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
}

// ---- Recording -------------------------------------------------------------------------------------------------

@Composable
private fun RecordingGlyph(state: WRecordingState, color: Color) {
    Canvas(Modifier.size(20.dp).clearAndSetSemantics {}) {
        val w = size.width
        when (state) {
            WRecordingState.Recording -> drawCircle(color, w / 2)
            WRecordingState.Paused -> {
                drawRect(color, Offset(w * 0.2f, w * 0.15f), Size(w * 0.22f, w * 0.7f))
                drawRect(color, Offset(w * 0.58f, w * 0.15f), Size(w * 0.22f, w * 0.7f))
            }
            WRecordingState.Processing -> drawArc(color, 0f, 270f, false, style = Stroke(w * 0.14f), topLeft = Offset(w * 0.1f, w * 0.1f), size = Size(w * 0.8f, w * 0.8f))
            WRecordingState.Off -> drawCircle(color, w / 2 - w * 0.07f, style = Stroke(w * 0.14f))
            is WRecordingState.Problem -> {
                drawLine(color, Offset(w / 2, w * 0.1f), Offset(w / 2, w * 0.62f), w * 0.16f)
                drawCircle(color, w * 0.09f, Offset(w / 2, w * 0.85f))
            }
        }
    }
}

/**
 * The recording state, always on screen while a recording exists. As a bar it runs the full width with
 * Pause and Stop; as a pill it is the compact form shown above the navigation bar on other screens.
 */
@Composable
fun WRecordingIndicator(
    state: WRecordingState,
    elapsedSeconds: Int,
    modifier: Modifier = Modifier,
    pill: Boolean = false,
    onPauseResume: (() -> Unit)? = null,
    onStop: (() -> Unit)? = null,
) {
    val strings = LocalWStrings.current
    val scheme = MaterialTheme.colorScheme
    val filled = state == WRecordingState.Recording
    val tint = when (state) {
        WRecordingState.Recording -> fixed(WFixed.recording)
        WRecordingState.Paused -> fixed(WFixed.paused)
        is WRecordingState.Problem -> scheme.error
        else -> scheme.onSurfaceVariant
    }
    val content = if (filled) fixed(WFixed.onRecording) else tint
    val shape = if (pill) CircleShape else cardShape
    Column(
        modifier.fillMaxWidth().heightIn(min = WTokens.Size.target).clip(shape)
            .background(if (filled) fixed(WFixed.recording) else scheme.surfaceVariant)
            .then(if (filled) Modifier else Modifier.border(1.5.dp, tint, shape))
            .padding(horizontal = S.lg, vertical = S.sm),
        verticalArrangement = Arrangement.spacedBy(S.sm),
    ) {
        Row(
            Modifier.fillMaxWidth().heightIn(min = WTokens.Size.target - S.sm * 2).clearAndSetSemantics {
                contentDescription = state.spokenLabel(elapsedSeconds, strings)
                if (!state.isLive) liveRegion = LiveRegionMode.Polite
            },
            verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.sm),
        ) {
            RecordingGlyph(state, content)
            Column(Modifier.weight(1f)) {
                Text(state.word(strings), style = T.bodyStrong.style(), color = content)
                if (state is WRecordingState.Problem) Text(state.reason, style = T.caption.style(), color = content)
            }
            if (state.isLive) Text(WElapsed.clock(elapsedSeconds), style = (if (pill) T.time else T.title).tabular(), color = content)
        }
        // Pause and Stop sit on their own row so the state word and the time never wrap, at any font scale.
        if (!pill && state.isLive && (onPauseResume != null || onStop != null)) {
            CompositionLocalProvider(LocalContentColor provides content) {
                val colors = ButtonDefaults.outlinedButtonColors(contentColor = content)
                val stroke = BorderStroke(1.dp, content)
                Row(horizontalArrangement = Arrangement.spacedBy(S.sm)) {
                    onPauseResume?.let {
                        OutlinedButton(it, Modifier.weight(1f).heightIn(min = WTokens.Size.target), colors = colors, border = stroke) {
                            Text(if (state == WRecordingState.Paused) strings.resume else strings.pause, style = T.bodyStrong.style())
                        }
                    }
                    onStop?.let {
                        OutlinedButton(it, Modifier.weight(1f).heightIn(min = WTokens.Size.target), colors = colors, border = stroke) { Text(strings.stop, style = T.bodyStrong.style()) }
                    }
                }
            }
        }
    }
}

/** Shown at the start of every in-person recording. It never says anyone was told or agreed. */
@Composable
fun WConsentBanner(onShowNotice: () -> Unit, onDone: () -> Unit, modifier: Modifier = Modifier) {
    val strings = LocalWStrings.current
    Column(
        modifier.fillMaxWidth().clip(cardShape).background(fixed(WFixed.recordingTint)).padding(S.md),
        verticalArrangement = Arrangement.spacedBy(S.sm),
    ) {
        Text(strings.consentMessage, style = T.body.style(), color = MaterialTheme.colorScheme.onSurface)
        Row(horizontalArrangement = Arrangement.spacedBy(S.sm)) {
            WButton(strings.showNotice, onShowNotice, Modifier.weight(1f), WButtonKind.Secondary)
            WButton(strings.done, onDone, Modifier.weight(1f), WButtonKind.Secondary)
        }
    }
}

// ---- Thread ----------------------------------------------------------------------------------------------------

enum class WSpineNode { Upcoming, Now, Past, Recording }

/**
 * One stop on the time line: a time label and node on the left, content beside it. The connector is
 * dashed ahead of now and solid behind it.
 */
/**
 * A time for the thread's narrow column. Some phones format "10:00 PM" with a no-break space, which makes it one
 * word; too wide for the column, it then broke inside "PM". With an ordinary space it wraps as "10:00" over "PM".
 */
/** From this system text scale up, a thread stop puts its time above its card instead of beside it. */
internal const val SPINE_STACKS_AT = 1.5f

internal fun spineTimeLabel(time: String): String = time.replace('\u202F', ' ').replace('\u00A0', ' ')

@Composable
fun WSpineStop(time: String, modifier: Modifier = Modifier, node: WSpineNode = WSpineNode.Upcoming, showsNode: Boolean = true, content: @Composable () -> Unit) {
    val accent = MaterialTheme.colorScheme.primary
    val page = MaterialTheme.colorScheme.background
    val nodeColor = if (node == WSpineNode.Recording) fixed(WFixed.recording) else accent
    // At the largest text sizes the time no longer fits the narrow column ("10:0" over "0 PM"), and widening the
    // column squeezed the card until long words broke. So the time sits above the card, which keeps the full
    // width, as on the iPhone. The line and its node are decoration and are left out.
    if (LocalDensity.current.fontScale >= SPINE_STACKS_AT) {
        Column(modifier.fillMaxWidth().padding(bottom = S.sm), verticalArrangement = Arrangement.spacedBy(S.xs)) {
            Text(
                spineTimeLabel(time).replace('\n', ' '),
                style = (if (node == WSpineNode.Past) T.caption else T.time).tabular(),
                color = if (node == WSpineNode.Past) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
            )
            content()
        }
        return
    }
    Row(modifier.fillMaxWidth().height(IntrinsicSize.Min), horizontalArrangement = Arrangement.spacedBy(S.sm)) {
        Text(
            spineTimeLabel(time), Modifier.width(WTokens.Size.spine - 8.dp).padding(top = S.md),
            style = (if (node == WSpineNode.Past) T.caption else T.time).tabular(), textAlign = TextAlign.End,
            color = if (node == WSpineNode.Past) MaterialTheme.colorScheme.onSurfaceVariant else MaterialTheme.colorScheme.onSurface,
        )
        Canvas(Modifier.width(12.dp).fillMaxHeight().clearAndSetSemantics {}) {
            val x = size.width / 2
            if (node == WSpineNode.Past) {
                drawLine(accent.copy(alpha = 0.5f), Offset(x, 4.dp.toPx()), Offset(x, size.height), 4.dp.toPx())
            } else {
                drawLine(accent.copy(alpha = 0.5f), Offset(x, 0f), Offset(x, size.height), 2.dp.toPx(), pathEffect = PathEffect.dashPathEffect(floatArrayOf(4.dp.toPx(), 4.dp.toPx())))
            }
            if (showsNode) {
                val center = Offset(x, S.md.toPx() + 8.dp.toPx())
                val hollow = node == WSpineNode.Upcoming || node == WSpineNode.Past
                drawCircle(if (hollow) page else nodeColor, 6.dp.toPx(), center)
                drawCircle(nodeColor, 5.dp.toPx(), center, style = Stroke(2.dp.toPx()))
            }
        }
        Box(Modifier.weight(1f).padding(bottom = S.sm)) { content() }
    }
}

/** The "Now" pill that divides what is coming from what has happened. */
@Composable
fun WNowMarker(time: String, modifier: Modifier = Modifier) {
    val strings = LocalWStrings.current
    Box(modifier.fillMaxWidth(), contentAlignment = Alignment.Center) {
        Text(
            "${strings.now} · $time",
            Modifier.clip(CircleShape).background(MaterialTheme.colorScheme.primary).padding(horizontal = S.md, vertical = S.xs).semantics { heading() },
            style = T.time.tabular(), color = MaterialTheme.colorScheme.onPrimary,
        )
    }
}

enum class WMeetingVariant { Next, Upcoming, Past }

/** A meeting on the thread. Next is the large card with actions; the others are rows. */
@OptIn(ExperimentalLayoutApi::class)
@Composable
fun WMeetingCard(
    variant: WMeetingVariant,
    title: String,
    modifier: Modifier = Modifier,
    status: String? = null,
    detail: String? = null,
    attendees: List<String> = emptyList(),
    actions: (@Composable RowScope.() -> Unit)? = null,
) {
    val scheme = MaterialTheme.colorScheme
    val next = variant == WMeetingVariant.Next
    WCard(modifier, raised = next) {
        status?.let {
            Text(it, Modifier.clip(controlShape).background(scheme.tertiaryContainer).padding(horizontal = S.sm, vertical = S.xxs), style = T.caption.style().copy(fontWeight = FontWeight.Medium), color = scheme.primary)
        }
        Text(title, style = (if (next) T.title else T.subtitle).style())
        detail?.let { Text(it, style = T.secondary.style(), color = scheme.onSurfaceVariant) }
        if (next && attendees.isNotEmpty()) {
            FlowRow(horizontalArrangement = Arrangement.spacedBy(S.xs), verticalArrangement = Arrangement.spacedBy(S.xs)) { attendees.forEach { WSpeakerChip(it) } }
        }
        actions?.let { Row(Modifier.padding(top = S.xs), horizontalArrangement = Arrangement.spacedBy(S.sm), content = it) }
    }
}

enum class WAppDot { Blue, Green, Purple, Amber }

data class WMemoryItem(val dot: WAppDot, val text: String)

/**
 * A stretch of work the desktops saw: a time range, the device, and the apps and items in it. Each item
 * has a dot and always its app's name; the dot color is decoration.
 */
@Composable
fun WMemoryBlock(range: String, items: List<WMemoryItem>, modifier: Modifier = Modifier, device: String? = null) {
    val strings = LocalWStrings.current
    WCard(modifier.semantics(mergeDescendants = true) {}) {
        Text(listOfNotNull(range, strings.memory, device).joinToString(" · "), style = T.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
        items.forEach { item ->
            val dot = fixed(when (item.dot) { WAppDot.Blue -> WFixed.appBlue; WAppDot.Green -> WFixed.appGreen; WAppDot.Purple -> WFixed.appPurple; WAppDot.Amber -> WFixed.appAmber })
            Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.sm)) {
                Box(Modifier.size(8.dp).background(dot, CircleShape))
                Text(item.text, style = T.secondary.style())
            }
        }
    }
}

enum class WLineState { Normal, Cited, Live }

/** One line of a transcript: a time, an optional speaker (phone recordings have none), the text. */
@Composable
fun WTranscriptLine(time: String, text: String, modifier: Modifier = Modifier, speaker: String? = null, state: WLineState = WLineState.Normal) {
    val strings = LocalWStrings.current
    val scheme = MaterialTheme.colorScheme
    Column(
        modifier.fillMaxWidth().clip(controlShape).background(if (state == WLineState.Cited) fixed(WFixed.aiTint) else Color.Transparent)
            .padding(S.sm).semantics(mergeDescendants = true) {},
        verticalArrangement = Arrangement.spacedBy(S.xs),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.sm)) {
            Text(time, style = T.caption.tabular(), color = scheme.onSurfaceVariant)
            speaker?.let { WSpeakerChip(it) }
            if (state == WLineState.Cited) Text(strings.cited, style = T.caption.style().copy(fontWeight = FontWeight.Medium), color = fixed(WFixed.aiInk))
        }
        Text(text, style = T.body.style(), color = scheme.onSurface)
        if (state == WLineState.Live) Text(strings.listening, style = T.caption.style(), color = scheme.onSurfaceVariant)
    }
}

// ---- Notes -----------------------------------------------------------------------------------------------------

/** The small numbered circle on a line Whimbrel wrote. A button: activating it selects its source. */
@Composable
fun WSourceMark(number: Int, onClick: () -> Unit, modifier: Modifier = Modifier, selected: Boolean = false) {
    val strings = LocalWStrings.current
    val ink = fixed(WFixed.aiInk)
    val card = MaterialTheme.colorScheme.surfaceVariant
    Box(
        modifier.size(WTokens.Size.target).clickable(role = Role.Button, onClick = onClick)
            .semantics { contentDescription = strings.sourceNumber(number); this.selected = selected },
        contentAlignment = Alignment.Center,
    ) {
        Box(Modifier.size(24.dp).background(if (selected) ink else card, CircleShape).border(1.5.dp, ink, CircleShape), contentAlignment = Alignment.Center) {
            Text("$number", style = T.caption.tabular().copy(fontWeight = FontWeight.Bold), color = if (selected) card else ink)
        }
    }
}

enum class WNoteKind { Heading, Paragraph, Bullet }
enum class WAuthor { You, Whimbrel }

/**
 * One block of a note. Your words are in the text color; Whimbrel's are in AI ink with a left rule and a
 * source mark, so the two never look alike.
 */
@Composable
fun WNoteBlock(
    kind: WNoteKind,
    author: WAuthor,
    text: String,
    modifier: Modifier = Modifier,
    source: Int? = null,
    selected: Boolean = false,
    onSource: (() -> Unit)? = null,
) {
    val strings = LocalWStrings.current
    val scheme = MaterialTheme.colorScheme
    val ai = author == WAuthor.Whimbrel
    val ink = if (ai) fixed(WFixed.aiInk) else scheme.onSurface
    Row(
        modifier.fillMaxWidth().height(IntrinsicSize.Min).background(if (selected) fixed(WFixed.aiTint) else Color.Transparent).padding(vertical = S.xs),
        horizontalArrangement = Arrangement.spacedBy(S.sm),
    ) {
        if (ai && kind != WNoteKind.Heading) Box(Modifier.width(2.dp).fillMaxHeight().background(ink))
        if (kind == WNoteKind.Bullet) Box(Modifier.padding(top = 11.dp).size(5.dp).background(ink, CircleShape))
        if (kind == WNoteKind.Heading) {
            Text(text.uppercase(), Modifier.weight(1f).semantics { heading() }, style = T.caption.style().copy(fontWeight = FontWeight.Bold), color = if (ai) ink else scheme.onSurfaceVariant)
        } else {
            Text(text, Modifier.weight(1f).semantics { if (ai) contentDescription = strings.whimbrelWrote(text) }, style = T.reading.reading(), color = ink)
        }
        if (source != null && onSource != null) WSourceMark(source, onSource, selected = selected)
    }
}

enum class WSourceKind { TranscriptMoment, SeenOnScreen }

/** Where a line came from: its number, kind, title, time, a short quote and Open. */
@Composable
fun WSourceCard(number: Int, kind: WSourceKind, title: String, time: String, onOpen: () -> Unit, modifier: Modifier = Modifier, quote: String? = null, selected: Boolean = false) {
    val strings = LocalWStrings.current
    val ink = fixed(WFixed.aiInk)
    WCard(modifier, tint = if (selected) fixed(WFixed.aiTint) else null, border = if (selected) ink else null) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(S.sm)) {
            Box(Modifier.size(24.dp).background(ink, CircleShape).semantics { contentDescription = strings.sourceNumber(number) }, contentAlignment = Alignment.Center) {
                Text("$number", Modifier.clearAndSetSemantics {}, style = T.caption.tabular().copy(fontWeight = FontWeight.Bold), color = MaterialTheme.colorScheme.surfaceVariant)
            }
            Text("${if (kind == WSourceKind.TranscriptMoment) strings.transcriptMoment else strings.seenOnScreen} · $time", style = T.caption.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
        Text(title, style = T.subtitle.style())
        quote?.let { Text(it, style = T.secondary.style()) }
        WButton(strings.open, onOpen, kind = WButtonKind.Secondary)
    }
}

/** An action item: a checkbox, the text, who owns it and when it is due. The whole row toggles. */
@Composable
fun WActionItemRow(text: String, done: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier, owner: String? = null, due: String? = null) {
    val strings = LocalWStrings.current
    val scheme = MaterialTheme.colorScheme
    Row(
        modifier.fillMaxWidth().heightIn(min = WTokens.Size.target).clickable(role = Role.Checkbox, onClick = onToggle)
            .semantics(mergeDescendants = true) { stateDescription = if (done) strings.done else strings.notDone }.padding(vertical = S.xs),
        horizontalArrangement = Arrangement.spacedBy(S.md),
    ) {
        Box(
            Modifier.padding(top = 2.dp).size(22.dp).background(if (done) scheme.primary else Color.Transparent, RoundedCornerShape(3.dp))
                .border(2.dp, if (done) scheme.primary else scheme.onSurfaceVariant, RoundedCornerShape(3.dp)),
            contentAlignment = Alignment.Center,
        ) { if (done) Text("✓", Modifier.clearAndSetSemantics {}, style = T.caption.style().copy(fontWeight = FontWeight.Bold), color = scheme.onPrimary) }
        Column(Modifier.weight(1f), verticalArrangement = Arrangement.spacedBy(S.xxs)) {
            Text(text, style = T.body.style().copy(textDecoration = if (done) TextDecoration.LineThrough else null), color = if (done) scheme.onSurfaceVariant else scheme.onSurface)
            val meta = listOfNotNull(owner, due).joinToString(" · ")
            if (meta.isNotEmpty()) Text(meta, style = T.secondary.style(), color = scheme.onSurfaceVariant)
        }
    }
}

// ---- States ----------------------------------------------------------------------------------------------------

enum class WStateKind { Empty, Loading, Error }

/** Empty, loading and error share one shape: a one-line title, one line of help, an optional action. */
@Composable
fun WStateView(kind: WStateKind, title: String, modifier: Modifier = Modifier, help: String? = null, actionTitle: String? = null, onAction: (() -> Unit)? = null) {
    val strings = LocalWStrings.current
    val scheme = MaterialTheme.colorScheme
    Column(modifier.fillMaxWidth().padding(S.xl), horizontalAlignment = Alignment.CenterHorizontally, verticalArrangement = Arrangement.spacedBy(S.md)) {
        when (kind) {
            WStateKind.Loading -> CircularProgressIndicator(Modifier.size(28.dp))
            WStateKind.Error -> RecordingGlyph(WRecordingState.Problem(""), scheme.error)
            WStateKind.Empty -> RecordingGlyph(WRecordingState.Off, scheme.onSurfaceVariant)
        }
        Text(title, style = T.subtitle.style(), textAlign = TextAlign.Center, color = scheme.onSurface)
        help?.let { Text(it, style = T.secondary.style(), textAlign = TextAlign.Center, color = scheme.onSurfaceVariant) }
        onAction?.let { WButton(actionTitle ?: strings.tryAgain, it, kind = WButtonKind.Secondary) }
    }
}

/** A line across the top of a screen when there is no connection. Synced content stays usable below it. */
@Composable
fun WOfflineBanner(modifier: Modifier = Modifier) {
    Text(
        LocalWStrings.current.offline,
        modifier.fillMaxWidth().clip(controlShape).background(LocalThemePalette.current.highlight).padding(S.md).semantics { liveRegion = LiveRegionMode.Polite },
        style = T.secondary.style(), color = MaterialTheme.colorScheme.onSurface,
    )
}

