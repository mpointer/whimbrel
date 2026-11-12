package com.mpointer.whimbrel.mobile.ui

import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.selection.selectable
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Check
import androidx.compose.material3.Button
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.ModalBottomSheet
import androidx.compose.material3.Switch
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.BuildConfig
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.read.ReadLogic
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.ThemeStore
import kotlinx.coroutines.launch

/** This phone's name, the sync line with Sync now, the theme choice (per device) and the app version. Ported from `SettingsSheet.swift`. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun SettingsSheet(sync: SyncModel, reminders: ReminderCoordinator, themes: ThemeStore, phoneName: String, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val syncing by sync.isSyncing.collectAsState()
    val installed by themes.themes.collectAsState()
    val selectedId by themes.selectedId.collectAsState()
    val syncLine = rememberSyncLine(sync)
    val olderLoading by sync.olderHistoryLoading.collectAsState()
    val count by sync.meetingCount.collectAsState()
    val countLine = ReadLogic.meetingCountLine(count, ResourceReadStrings(LocalContext.current))
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.settings_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
            Heading(stringResource(R.string.settings_this_phone))
            Text(phoneName)
            Heading(stringResource(R.string.settings_sync))
            Text(syncLine, color = MaterialTheme.colorScheme.onSurfaceVariant)
            if (olderLoading) Text(stringResource(R.string.sync_older_history_loading), color = MaterialTheme.colorScheme.onSurfaceVariant)
            countLine?.let { Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            Button(onClick = { scope.launch { sync.syncNow(manual = true) } }, enabled = !syncing) { Text(stringResource(R.string.settings_sync_now)) }
            RemindersSection(reminders)
            Heading(stringResource(R.string.settings_theme))
            installed.forEach { theme ->
                val selected = theme.id == selectedId
                Row(
                    Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(selected = selected, role = Role.RadioButton, onClick = { themes.select(theme.id) }),
                    verticalAlignment = Alignment.CenterVertically,
                    horizontalArrangement = Arrangement.SpaceBetween,
                ) {
                    Text(theme.name)
                    if (selected) Icon(Icons.Filled.Check, stringResource(R.string.settings_theme_selected), tint = MaterialTheme.colorScheme.primary)
                }
            }
            TextButton(onClick = onDismiss, modifier = Modifier.align(Alignment.End)) { Text(stringResource(R.string.done)) }
            Text(stringResource(R.string.version_label, BuildConfig.VERSION_NAME), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp))
        }
    }
}

@Composable
private fun Heading(text: String) =
    Text(text.uppercase(), style = MaterialTheme.typography.labelSmall, fontWeight = FontWeight.SemiBold, color = MaterialTheme.colorScheme.onSurfaceVariant, modifier = Modifier.padding(top = 8.dp).semantics { heading() })

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
    Heading(stringResource(R.string.reminders_section))
    if (rows.showsUpgradeLines) {
        Text(stringResource(R.string.reminders_need_cloud))
        Text(stringResource(R.string.reminders_upgrade), color = MaterialTheme.colorScheme.onSurfaceVariant)
    }
    if (rows.showsMeetingToggle) {
        ToggleRow(stringResource(R.string.reminders_meeting_toggle), current.meetingEnabled) { change(current.copy(meetingEnabled = it)) }
    }
    if (rows.showsLeadPicker) {
        Text(stringResource(R.string.reminders_lead_label), color = MaterialTheme.colorScheme.onSurfaceVariant)
        listOf(5, 10, 15).forEach { minutes ->
            val selected = current.leadMinutes == minutes
            Row(
                Modifier.fillMaxWidth().heightIn(min = 48.dp).selectable(selected = selected, role = Role.RadioButton, onClick = { change(current.copy(leadMinutes = minutes)) }),
                verticalAlignment = Alignment.CenterVertically,
                horizontalArrangement = Arrangement.SpaceBetween,
            ) {
                Text(pluralStringResource(R.plurals.reminders_lead_minutes, minutes, minutes))
                if (selected) Icon(Icons.Filled.Check, stringResource(R.string.settings_theme_selected), tint = MaterialTheme.colorScheme.primary)
            }
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
            Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable {
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
            Text(stringResource(R.string.reminders_digest_at))
            Text(label, color = MaterialTheme.colorScheme.primary)
        }
    }
    if (rows.showsDeniedNotice) Text(stringResource(R.string.reminders_denied), color = MaterialTheme.colorScheme.onSurfaceVariant)
    if (rows.showsFooter) Text(stringResource(R.string.reminders_footer), style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
}

@Composable
private fun ToggleRow(label: String, checked: Boolean, onChange: (Boolean) -> Unit) {
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp), verticalAlignment = Alignment.CenterVertically, horizontalArrangement = Arrangement.SpaceBetween) {
        Text(label)
        Switch(checked = checked, onCheckedChange = onChange)
    }
}
