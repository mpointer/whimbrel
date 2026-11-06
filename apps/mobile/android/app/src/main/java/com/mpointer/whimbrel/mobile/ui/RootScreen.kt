package com.mpointer.whimbrel.mobile.ui

import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.WindowInsetsSides
import androidx.compose.foundation.layout.navigationBars
import androidx.compose.foundation.layout.only
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Home
import androidx.compose.material.icons.automirrored.filled.List
import androidx.compose.material.icons.filled.Refresh
import androidx.compose.material3.Icon
import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.currentWindowAdaptiveInfo
import androidx.compose.material3.adaptive.layout.ListDetailPaneScaffoldRole
import androidx.compose.material3.adaptive.layout.ThreePaneScaffoldDestinationItem
import androidx.compose.material3.adaptive.navigation.rememberListDetailPaneScaffoldNavigator
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteScaffold
import androidx.compose.material3.adaptive.navigationsuite.NavigationSuiteType
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.collectAsState
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.rememberCoroutineScope
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.material3.Text
import androidx.compose.ui.Modifier
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.res.stringResource
import com.mpointer.whimbrel.mobile.R
import com.mpointer.whimbrel.mobile.read.MeetingsModel
import com.mpointer.whimbrel.mobile.read.RoutinesModel
import com.mpointer.whimbrel.mobile.read.TodayModel
import com.mpointer.whimbrel.mobile.sync.EnrollmentModel
import com.mpointer.whimbrel.mobile.sync.SyncModel
import com.mpointer.whimbrel.mobile.theme.ThemeStore
import kotlinx.coroutines.launch

enum class RootTab(val icon: ImageVector, val label: Int) {
    Today(Icons.Filled.Home, R.string.tab_today),
    Meetings(Icons.AutoMirrored.Filled.List, R.string.tab_meetings),
    Routines(Icons.Filled.Refresh, R.string.tab_routines),
}

/** Where the shell starts; only a debug demo launch sets anything but the default. */
data class RootStart(val tab: RootTab = RootTab.Today, val meetingId: Long? = null, val settingsOpen: Boolean = false, val launchId: Int = 0)

/**
 * The approved phone: Today, Meetings and Routines in a bottom bar (cover screen) or a rail (inner screen),
 * with Settings in a sheet. The tested [navKind], not the library default, picks which.
 */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
@Composable
fun RootScreen(
    sync: SyncModel,
    enrollment: EnrollmentModel,
    themes: ThemeStore,
    phoneName: String,
    start: RootStart = RootStart(),
) {
    var tab by rememberSaveable { mutableStateOf(start.tab) }
    // True while the open meeting was reached from a Today row: closing it returns to Today, as the iPhone's pushed detail does.
    var fromToday by rememberSaveable { mutableStateOf(false) }
    var settingsOpen by rememberSaveable { mutableStateOf(start.settingsOpen) }
    val today = remember { TodayModel() }
    val meetings = remember { MeetingsModel() }
    val routines = remember { RoutinesModel() }
    val scope = rememberCoroutineScope() // Main
    val info = currentWindowAdaptiveInfo()
    // The open meeting is the navigator's current Detail: saved across process death, kept across fold and unfold.
    val navigator = rememberListDetailPaneScaffoldNavigator<Any>(
        scaffoldDirective = adaptiveDirective(info),
        initialDestinationHistory = listOfNotNull(
            ThreePaneScaffoldDestinationItem(ListDetailPaneScaffoldRole.List),
            start.meetingId?.let { ThreePaneScaffoldDestinationItem(ListDetailPaneScaffoldRole.Detail, it) },
        ),
    )
    val detailOpen = navigator.currentDestination?.pane == ListDetailPaneScaffoldRole.Detail
    var wasOpen by remember { mutableStateOf(false) }
    // True while a new selection replaces the open one: its pop is not a close, so it must not return to Today.
    var replacing by remember { mutableStateOf(false) }
    val openMeeting: (Long) -> Unit = { id ->
        scope.launch {
            replacing = true
            try { navigator.openMeeting(id) } finally { replacing = false }
        }
    }
    // A later demo launch (launchId > 0) arriving in the running Activity; the first one is the initial state above.
    LaunchedEffect(start.launchId) {
        if (start.launchId > 0) {
            fromToday = false
            tab = start.tab
            settingsOpen = start.settingsOpen
            start.meetingId?.let(openMeeting)
        }
    }
    LaunchedEffect(detailOpen) {
        if (wasOpen && !detailOpen && !replacing && fromToday && tab == RootTab.Meetings) { tab = RootTab.Today; fromToday = false }
        wasOpen = detailOpen
    }
    // Once per sync, as on the iPhone: names the devices that recorded each meeting.
    val revision by sync.readRevision.collectAsState()
    LaunchedEffect(revision) { enrollment.refreshDeviceLabels() }
    val kind = navKind(info.widthDp)

    NavigationSuiteScaffold(
        navigationSuiteItems = {
            RootTab.entries.forEach { t ->
                item(
                    selected = t == tab,
                    onClick = { tab = t; fromToday = false },
                    icon = { Icon(t.icon, contentDescription = null) },
                    label = { Text(stringResource(t.label)) },
                )
            }
        },
        layoutType = if (kind == NavKind.Rail) NavigationSuiteType.NavigationRail else NavigationSuiteType.NavigationBar,
    ) {
        // A rail leaves the bottom edge to the content; a bar already sits above the gesture area.
        Box(if (kind == NavKind.Rail) Modifier.windowInsetsPadding(WindowInsets.navigationBars.only(WindowInsetsSides.Bottom)) else Modifier) {
            when (tab) {
                RootTab.Today -> TodayScreen(
                    sync = sync,
                    model = today,
                    labels = enrollment.deviceLabels,
                    onOpenSettings = { settingsOpen = true },
                    onOpenMeeting = { id ->
                        fromToday = true
                        tab = RootTab.Meetings
                        openMeeting(id)
                    },
                )
                RootTab.Meetings -> MeetingsScreen(sync, meetings, enrollment.deviceLabels, navigator, openMeeting)
                RootTab.Routines -> RoutinesScreen(sync, routines, enrollment.deviceLabels)
            }
        }
    }
    if (settingsOpen) SettingsSheet(sync, themes, phoneName, onDismiss = { settingsOpen = false })
}
