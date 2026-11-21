package com.mpointer.whimbrel.mobile.ui

import android.content.Context
import android.content.SharedPreferences
import android.text.format.DateFormat as AndroidDateFormat
import java.util.Date
import java.util.TimeZone
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.material.icons.automirrored.filled.ArrowBack
import androidx.compose.material.icons.automirrored.filled.KeyboardArrowRight
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.IconButton
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.ui.draw.clip
import androidx.lifecycle.repeatOnLifecycle
import com.mpointer.whimbrel.mobile.design.WStateKind
import com.mpointer.whimbrel.mobile.design.WStateView
import com.mpointer.whimbrel.mobile.sync.DeviceApproval
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.rememberModalBottomSheetState
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.setValue
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.core.app.NotificationManagerCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.LifecycleEventObserver
import androidx.lifecycle.ProcessLifecycleOwner
import com.mpointer.whimbrel.mobile.reminders.ReminderCoordinator
import com.mpointer.whimbrel.mobile.reminders.ReminderSettings
import com.mpointer.whimbrel.mobile.reminders.ReminderSettingsRows
import androidx.compose.ui.res.pluralStringResource
import androidx.compose.ui.res.stringResource
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.heading
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.BuildConfig
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.design.WButton
import com.mpointer.whimbrel.mobile.design.WButtonKind
import com.mpointer.whimbrel.mobile.design.WCard
import com.mpointer.whimbrel.mobile.design.WTokens
import com.mpointer.whimbrel.mobile.design.style
import com.mpointer.whimbrel.mobile.read.ReadLogic
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.ThemeStore
import com.mpointer.whimbrel.mobile.update.UpdateModel
import com.mpointer.whimbrel.mobile.update.UpdateStatus
import com.mpointer.whimbrel.mobile.update.UpdateStatusStrings
import com.mpointer.whimbrel.mobile.update.UpdateTimeFormats
import kotlinx.coroutines.launch

/** This phone's name, the sync line with Sync now, the theme choice (per device) and the app version. Ported from `SettingsSheet.swift`. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSheet(
    sync: SyncModel, reminders: ReminderCoordinator, themes: ThemeStore, phoneName: String, enrollment: EnrollmentModel,
    onDismiss: () -> Unit, startOnDevices: Boolean = false,
) {
    var devicesOpen by rememberSaveable { mutableStateOf(startOnDevices) }
    val pendingCount by enrollment.pendingDeviceCount.collectAsState()
    val pending = if (DeviceApproval.enabled) pendingCount else 0
    val scope = rememberCoroutineScope() // Main
    val syncing by sync.isSyncing.collectAsState()
    val installed by themes.themes.collectAsState()
    val selectedId by themes.selectedId.collectAsState()
    val syncLine = rememberSyncLine(sync)
    val olderLoading by sync.olderHistoryLoading.collectAsState()
    val updateNeeded by sync.updateNeeded.collectAsState()
    val count by sync.meetingCount.collectAsState()
    val countLine = ReadLogic.meetingCountLine(count, ResourceReadStrings(LocalContext.current))
    // Opens at full height: the half-open sheet hid every section below Sync.
    ModalBottomSheet(onDismissRequest = onDismiss, sheetState = rememberModalBottomSheetState(skipPartiallyExpanded = true), containerColor = MaterialTheme.colorScheme.background) {
        if (devicesOpen) {
            androidx.activity.compose.BackHandler { devicesOpen = false }
            DevicesContent(enrollment, onBack = { devicesOpen = false })
            return@ModalBottomSheet
        }
        Column(
            Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl),
            verticalArrangement = Arrangement.spacedBy(WTokens.Space.sm),
        ) {
            Row(Modifier.fillMaxWidth(), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
                Text(stringResource(R.string.settings_title), style = WTokens.TypeScale.title.style(), modifier = Modifier.semantics { heading() })
                WButton(stringResource(R.string.done), onDismiss, kind = WButtonKind.Quiet)
            }
            Heading(stringResource(R.string.settings_this_phone))
            WCard { Text(phoneName, style = WTokens.TypeScale.body.style()) }
            Heading(stringResource(R.string.devices_title))
            val badge = if (pending > 0) stringResource(R.string.devices_badge, pending) else null
            WCard(Modifier.clip(RoundedCornerShape(WTokens.Radius.card)).clickable(role = Role.Button) { devicesOpen = true }) {
                Row(Modifier.fillMaxWidth().heightIn(min = WTokens.Size.target - WTokens.Space.md * 2), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
                    Column(Modifier.weight(1f)) {
                        Text(stringResource(R.string.devices_title), style = WTokens.TypeScale.body.style())
                        badge?.let { Text(it, style = WTokens.TypeScale.time.style(), color = MaterialTheme.colorScheme.primary) }
                    }
                    Icon(Icons.AutoMirrored.Filled.KeyboardArrowRight, null, tint = MaterialTheme.colorScheme.onSurfaceVariant)
                }
            }
            Heading(stringResource(R.string.settings_sync))
            WCard {
                Note(syncLine)
                if (olderLoading) Note(stringResource(R.string.sync_older_history_loading))
                if (updateNeeded) Note(stringResource(R.string.sync_update_needed))
                countLine?.let { Note(it) }
                WButton(stringResource(R.string.settings_sync_now), { scope.launch { sync.syncNow(manual = true) } }, kind = WButtonKind.Secondary, enabled = !syncing)
            }
                    Heading(stringResource(R.string.reminders_section))
            WCard { RemindersSection(reminders) }
            Heading(stringResource(R.string.settings_theme))
            WCard {
                installed.forEach { theme ->
                    ChoiceRow(theme.name, theme.id == selectedId) { themes.select(theme.id) }
                }
            }
            Text(
                stringResource(R.string.version_label, BuildConfig.VERSION_NAME), style = WTokens.TypeScale.secondary.style(),
                color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = WTokens.Space.sm),
            )
            UpdateStatusLines()
        }
    }
}

/** The last update check (time and result) and, when the phone won't show them, a note that update notifications are off. */
@Composable
private fun UpdateStatusLines() {
    val context = LocalContext.current
    val prefs = remember { context.getSharedPreferences(UpdateModel.PREFS, Context.MODE_PRIVATE) }
    val strings = remember { ResourceUpdateStatusStrings(context) }
    val formats = remember { PhoneTimeFormats(context) }
    var last by remember { mutableStateOf(UpdateModel.lastCheck(prefs)) }
    var enabled by remember { mutableStateOf(NotificationManagerCompat.from(context).areNotificationsEnabled()) }
    val owner = remember { ProcessLifecycleOwner.get() }
    DisposableEffect(owner, prefs) {
        // The check that starts when the app resumes (or the background one) may finish while the sheet is open.
        val listener = SharedPreferences.OnSharedPreferenceChangeListener { _, _ -> last = UpdateModel.lastCheck(prefs) }
        prefs.registerOnSharedPreferenceChangeListener(listener)
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) {
                enabled = NotificationManagerCompat.from(context).areNotificationsEnabled()
                last = UpdateModel.lastCheck(prefs)
            }
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer); prefs.unregisterOnSharedPreferenceChangeListener(listener) }
    }
    Text(UpdateStatus.line(last, System.currentTimeMillis(), strings, formats), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
    if (!enabled) Text(stringResource(R.string.update_notifications_off), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

/** Times in the phone's own style: its 12/24-hour setting and its date order. */
private class PhoneTimeFormats(private val context: Context) : UpdateTimeFormats {
    override val zone: TimeZone get() = TimeZone.getDefault()
    override fun time(atMs: Long): String = AndroidDateFormat.getTimeFormat(context).format(Date(atMs))
    override fun dateTime(atMs: Long): String = AndroidDateFormat.getMediumDateFormat(context).format(Date(atMs)) + ", " + time(atMs)
}

private class ResourceUpdateStatusStrings(private val context: Context) : UpdateStatusStrings {
    override fun notChecked() = context.getString(R.string.update_status_never)
    override fun checkedOnly(time: String) = context.getString(R.string.update_status_checked_only, time)
    override fun upToDate(time: String) = context.getString(R.string.update_status_up_to_date, time)
    override fun updateAvailable(version: String, time: String) = context.getString(R.string.update_status_available, version, time)
    override fun network(time: String) = context.getString(R.string.update_status_network, time)
    override fun httpRateLimit(code: Int, time: String) = context.getString(R.string.update_status_http_rate_limit, code, time)
    override fun httpRefused(code: Int, time: String) = context.getString(R.string.update_status_http, code, time)
    override fun httpProblem(code: Int, time: String) = context.getString(R.string.update_status_http_problem, code, time)
    override fun unreadable(time: String) = context.getString(R.string.update_status_unreadable, time)
}

@Composable
private fun Heading(text: String) =
    Text(text, style = WTokens.TypeScale.subtitle.style(), color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = WTokens.Space.sm).semantics { heading() })

@Composable
private fun Note(text: String) = Text(text, style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)

/** One of a set of choices: the whole row is the radio button, and the selected one has a check mark. */
@Composable
private fun ChoiceRow(label: String, selected: Boolean, onClick: () -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = WTokens.Size.target).selectable(selected = selected, role = Role.RadioButton, onClick = onClick),
        verticalAlignment = Alignment.CenterVertically,
        horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, style = WTokens.TypeScale.body.style())
        if (selected) Icon(Icons.Filled.Check, null, tint = MaterialTheme.colorScheme.primary)
    }
}

/** The Reminders section, same rows and copy as the iPhone's. Every change is saved to the store and rescheduled. */
@Composable
private fun RemindersSection(coordinator: ReminderCoordinator) {
    val scope = rememberCoroutineScope() // Main
    val context = LocalContext.current
    val isCloud by coordinator.isCloud.collectAsState()
    var current by remember { mutableStateOf(coordinator.settings.settings) }
    // Refused notifications: re-read on return from the system settings. Before the prompt has ever been shown
    // Android reports the same "off", so the notice waits for it.
    var enabled by remember { mutableStateOf(NotificationManagerCompat.from(context).areNotificationsEnabled()) }
    val owner = remember { ProcessLifecycleOwner.get() }
    DisposableEffect(owner) {
        val observer = LifecycleEventObserver { _, event ->
            if (event == Lifecycle.Event.ON_RESUME) enabled = NotificationManagerCompat.from(context).areNotificationsEnabled()
        }
        owner.lifecycle.addObserver(observer)
        onDispose { owner.lifecycle.removeObserver(observer) }
    }
    val rows = ReminderSettingsRows.make(isCloud, current, denied = !enabled && coordinator.permissionAsked)
    val change: (ReminderSettings) -> Unit = { updated ->
        current = updated
        coordinator.settings.settings = updated
        scope.launch { coordinator.reschedule() }
    }
    if (rows.showsUpgradeLines) {
        Text(stringResource(R.string.reminders_need_cloud), style = WTokens.TypeScale.body.style())
        Note(stringResource(R.string.reminders_upgrade))
    }
    if (rows.showsMeetingToggle) {
        ToggleRow(stringResource(R.string.reminders_meeting_toggle), current.meetingEnabled) { change(current.copy(meetingEnabled = it)) }
    }
    if (rows.showsLeadPicker) {
        Note(stringResource(R.string.reminders_lead_label))
        listOf(5, 10, 15).forEach { minutes ->
            ChoiceRow(pluralStringResource(R.plurals.reminders_lead_minutes, minutes, minutes), current.leadMinutes == minutes) { change(current.copy(leadMinutes = minutes)) }
        }
    }
    if (rows.showsDigestToggle) {
        ToggleRow(stringResource(R.string.reminders_digest_toggle), current.digestEnabled) { change(current.copy(digestEnabled = it)) }
    }
    if (rows.showsDigestTime) {
        val minute = current.digestMinuteOfDay
        val label = remember(minute) {
            val cal = java.util.Calendar.getInstance().apply { set(java.util.Calendar.HOUR_OF_DAY, minute / 60); set(java.util.Calendar.MINUTE, minute % 60) }
            android.text.format.DateFormat.getTimeFormat(context).format(cal.time)
        }
        Row(
            Modifier.fillMaxWidth().heightIn(min = WTokens.Size.target).clickable(role = Role.Button) {
                android.app.TimePickerDialog(
                    context,
                    { _, hour, min -> change(current.copy(digestMinuteOfDay = hour * 60 + min)) },
                    minute / 60,
                    minute % 60,
                    android.text.format.DateFormat.is24HourFormat(context),
                ).show()
            },
            verticalAlignment = Alignment.CenterVertically,
            horizontalArrangement = Arrangement.SpaceBetween,
        ) {
            Text(stringResource(R.string.reminders_digest_at), style = WTokens.TypeScale.body.style())
            Text(label, style = WTokens.TypeScale.bodyStrong.style(), color = MaterialTheme.colorScheme.primary)
        }
    }
    if (rows.showsDeniedNotice) Note(stringResource(R.string.reminders_denied))
    if (rows.showsFooter) Note(stringResource(R.string.reminders_footer))
}

/** A labelled switch. The whole row toggles, and TalkBack reads the label and the state as one item. */
@Composable
private fun ToggleRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(
        Modifier.fillMaxWidth().heightIn(min = WTokens.Size.target).toggleable(value = checked, role = Role.Switch, onValueChange = onChange),
        verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween,
    ) {
        Text(label, Modifier.weight(1f), style = WTokens.TypeScale.body.style())
        Switch(checked = checked, onCheckedChange = null)
    }
}

/**
 * The account's other devices, and approving one that is waiting. The code line is in the text color, not
 * greyed, because it is the thing to read before tapping Approve; the button sits under it at full width. The
 * list is re-read every five seconds, only while this screen shows and the app is in front.
 */
@Composable
private fun DevicesContent(enrollment: EnrollmentModel, onBack: () -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val context = LocalContext.current
    val rows by enrollment.otherDevices.collectAsState()
    val approving by enrollment.approving.collectAsState()
    val error by enrollment.approvalError.collectAsState()
    var loaded by remember { mutableStateOf(false) }
    val owner = androidx.lifecycle.compose.LocalLifecycleOwner.current
    LaunchedEffect(owner) {
        owner.lifecycle.repeatOnLifecycle(Lifecycle.State.RESUMED) {
            while (true) {
                enrollment.refreshOtherDevices()
                loaded = true
                kotlinx.coroutines.delay(5_000)
            }
        }
    }
    Column(
        Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = WTokens.Space.lg).padding(bottom = WTokens.Space.xl),
        verticalArrangement = Arrangement.spacedBy(WTokens.Space.md),
    ) {
        Row(verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
            IconButton(onClick = onBack) { Icon(Icons.AutoMirrored.Filled.ArrowBack, stringResource(R.string.back)) }
            Text(stringResource(R.string.devices_title), style = WTokens.TypeScale.title.style(), modifier = Modifier.semantics { heading() })
        }
        error?.let { Problem(it.localized { id, args -> context.getString(id, *args) }) }
        if (rows.isEmpty()) {
            WStateView(if (loaded) WStateKind.Empty else WStateKind.Loading, stringResource(if (loaded) R.string.devices_none else R.string.today_loading))
        }
        rows.forEach { row ->
            WCard(raised = true) {
                Column(Modifier.semantics(mergeDescendants = true) { heading() }) {
                    Text(row.device.name?.takeIf { it.isNotBlank() } ?: row.device.platform ?: stringResource(R.string.devices_unnamed), style = WTokens.TypeScale.subtitle.style())
                    Text(deviceStatus(row.device.status), style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.onSurfaceVariant)
                }
                if (row.isPending && DeviceApproval.enabled) {
                    val code = row.code
                    if (code != null) Text(stringResource(R.string.devices_check_code, code), style = WTokens.TypeScale.body.style())
                    else Problem(stringResource(R.string.devices_cant_verify))
                }
                if (row.collides && DeviceApproval.enabled) Problem(stringResource(R.string.devices_collision))
                if (row.canApprove && DeviceApproval.enabled) {
                    WButton(stringResource(R.string.devices_approve), { scope.launch { enrollment.approve(row.device.id, row.device.publicKey) } }, Modifier.fillMaxWidth(), enabled = !approving)
                }
            }
        }
    }
}

/** The server's status word as something to read; a status this build does not know is shown as it is. */
@Composable
private fun deviceStatus(status: String): String = when (status) {
    "pending" -> stringResource(R.string.devices_status_pending)
    "approved" -> stringResource(R.string.devices_status_approved)
    "resetting" -> stringResource(R.string.devices_status_resetting)
    else -> status.replaceFirstChar(Char::uppercase)
}

/** Something went wrong, or needs care: a warning mark and the words, never color alone. */
@Composable
private fun Problem(text: String) {
    Row(Modifier.semantics(mergeDescendants = true) {}, horizontalArrangement = Arrangement.spacedBy(WTokens.Space.sm)) {
        Icon(Icons.Filled.Warning, null, Modifier.padding(top = 2.dp).size(textIconSize(18)), tint = MaterialTheme.colorScheme.error)
        Text(text, style = WTokens.TypeScale.secondary.style(), color = MaterialTheme.colorScheme.error)
    }
}
