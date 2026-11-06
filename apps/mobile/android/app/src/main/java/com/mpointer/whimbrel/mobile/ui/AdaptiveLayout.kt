package com.mpointer.whimbrel.mobile.ui

import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.WindowAdaptiveInfo
import androidx.compose.material3.adaptive.layout.PaneScaffoldDirective
import androidx.compose.material3.adaptive.layout.calculatePaneScaffoldDirective
import androidx.compose.ui.unit.dp
import androidx.window.core.layout.WindowWidthSizeClass

/** The Material window width classes. */
enum class Layout { Compact, Medium, Expanded }

fun layoutFor(widthDp: Int): Layout = when {
    widthDp < 600 -> Layout.Compact
    widthDp < 840 -> Layout.Medium
    else -> Layout.Expanded
}

/**
 * Two panes from 600 dp. The Fold's inner screen is Medium (about 670-750 dp), so the library's default
 * directive, which splits only at Expanded, is overridden by [adaptiveDirective].
 */
fun twoPane(widthDp: Int): Boolean = widthDp >= 600

enum class NavKind { BottomBar, Rail }

fun navKind(widthDp: Int): NavKind = if (layoutFor(widthDp) == Layout.Compact) NavKind.BottomBar else NavKind.Rail

fun horizontalPartitions(widthDp: Int): Int = if (twoPane(widthDp)) 2 else 1

/** The window width class's lower bound in dp (0, 600 or 840), which is exact at every threshold above. */
val WindowAdaptiveInfo.widthDp: Int
    get() = when (windowSizeClass.windowWidthSizeClass) {
        WindowWidthSizeClass.COMPACT -> 0
        WindowWidthSizeClass.MEDIUM -> 600
        else -> 840
    }

/** The default pane directive with its horizontal split moved to 600 dp; used by the list-detail scaffolds. */
@OptIn(ExperimentalMaterial3AdaptiveApi::class)
fun adaptiveDirective(info: WindowAdaptiveInfo): PaneScaffoldDirective =
    calculatePaneScaffoldDirective(info).let {
        val n = horizontalPartitions(info.widthDp)
        it.copy(maxHorizontalPartitions = n, horizontalPartitionSpacerSize = if (n == 2) 24.dp else it.horizontalPartitionSpacerSize)
    }
