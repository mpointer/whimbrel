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
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
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
fun SettingsSheet(sync: SyncModel, themes: ThemeStore, phoneName: String, onDismiss: () -> Unit) {
    val scope = rememberCoroutineScope() // Main
    val syncing by sync.isSyncing.collectAsState()
    val installed by themes.themes.collectAsState()
    val selectedId by themes.selectedId.collectAsState()
    val syncLine = rememberSyncLine(sync)
    val count by sync.meetingCount.collectAsState()
    val countLine = ReadLogic.meetingCountLine(count, ResourceReadStrings(LocalContext.current))
    ModalBottomSheet(onDismissRequest = onDismiss) {
        Column(Modifier.fillMaxWidth().verticalScroll(rememberScrollState()).padding(horizontal = 16.dp).padding(bottom = 24.dp), verticalArrangement = Arrangement.spacedBy(8.dp)) {
            Text(stringResource(R.string.settings_title), style = MaterialTheme.typography.titleLarge, modifier = Modifier.semantics { heading() })
            Heading(stringResource(R.string.settings_this_phone))
            Text(phoneName)
            Heading(stringResource(R.string.settings_sync))
            Text(syncLine, color = MaterialTheme.colorScheme.onSurfaceVariant)
            countLine?.let { Text(it, color = MaterialTheme.colorScheme.onSurfaceVariant) }
            Button(onClick = { scope.launch { sync.syncNow(manual = true) } }, enabled = !syncing) { Text(stringResource(R.string.settings_sync_now)) }
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
