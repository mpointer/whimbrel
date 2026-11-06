package com.mpointer.whimbrel.mobile.ui

import android.text.format.DateUtils
import androidx.compose.foundation.clickable
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.heightIn
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.statusBarsPadding
import androidx.compose.foundation.lazy.LazyColumn
import androidx.compose.foundation.lazy.items
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.CheckCircle
import androidx.compose.material.icons.filled.Warning
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Text
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
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextOverflow
import androidx.compose.ui.unit.dp
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.read.PhoneRoutineRun
import com.mpointer.whimbrel.mobile.read.ReadStrings
import com.mpointer.whimbrel.mobile.read.ResourceReadStrings
import com.mpointer.whimbrel.mobile.read.RoutinesModel
import com.mpointer.whimbrel.mobile.sync.DeviceLabels
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.LocalThemePalette
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.launch

private const val SUMMARY_LINES = 2

/** Routine runs from every device, newest first; read-only. Ported from `RoutinesView.swift`. Tap a row for its whole summary. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
fun RoutinesScreen(sync: SyncModel, model: RoutinesModel, labels: StateFlow<DeviceLabels>) {
    val scope = rememberCoroutineScope() // Main
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(revision) { model.reload(sync) }
    val runs by model.runs.collectAsState()
    val loaded by model.loaded.collectAsState()
    val deviceLabels by labels.collectAsState()
    val context = LocalContext.current
    val strings = remember(context) { ResourceReadStrings(context) }
    var refreshing by remember { mutableStateOf(false) }

    PullToRefreshBox(
        isRefreshing = refreshing,
        onRefresh = { scope.launch { refreshing = true; sync.syncNow(manual = true); refreshing = false } },
        modifier = Modifier.fillMaxSize().statusBarsPadding(),
    ) {
        LazyColumn(Modifier.fillMaxSize(), contentPadding = PaddingValues(16.dp), verticalArrangement = Arrangement.spacedBy(12.dp)) {
            items(runs, key = { it.id }) { RoutineRunRow(it, deviceLabels, strings) }
            if (runs.isEmpty()) item {
                Text(stringResource(if (loaded) R.string.routines_none else R.string.today_loading), color = MaterialTheme.colorScheme.onSurfaceVariant)
            }
        }
    }
}

@Composable
private fun RoutineRunRow(run: PhoneRoutineRun, labels: DeviceLabels, strings: ReadStrings) {
    var expanded by remember { mutableStateOf(false) }
    val caption = listOfNotNull(
        DateUtils.getRelativeTimeSpanString(run.ranAtUnixMs, System.currentTimeMillis(), DateUtils.MINUTE_IN_MILLIS).toString(),
        run.originDeviceId?.let { strings.device(labels.label(it)) },
    ).joinToString(" · ")
    Row(Modifier.fillMaxWidth().heightIn(min = 48.dp).clickable { expanded = !expanded }, horizontalArrangement = Arrangement.spacedBy(12.dp), verticalAlignment = Alignment.Top) {
        if (run.succeeded) {
            Icon(Icons.Filled.CheckCircle, stringResource(R.string.routines_succeeded), tint = LocalThemePalette.current.success)
        } else {
            Icon(Icons.Filled.Warning, stringResource(R.string.routines_failed), tint = MaterialTheme.colorScheme.error)
        }
        Column(verticalArrangement = Arrangement.spacedBy(2.dp)) {
            Text(run.routineName, style = MaterialTheme.typography.titleMedium, fontWeight = FontWeight.SemiBold)
            Text(
                run.outputSummary, color = MaterialTheme.colorScheme.onSurfaceVariant,
                maxLines = if (expanded) Int.MAX_VALUE else SUMMARY_LINES, overflow = TextOverflow.Ellipsis,
            )
            Text(caption, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.onSurfaceVariant)
        }
    }
}
