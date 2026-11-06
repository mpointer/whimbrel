package com.mpointer.whimbrel.mobile.ui

import androidx.compose.material3.adaptive.ExperimentalMaterial3AdaptiveApi
import androidx.compose.material3.adaptive.Posture
import androidx.compose.material3.adaptive.WindowAdaptiveInfo
import androidx.compose.ui.unit.dp
import androidx.window.core.layout.WindowSizeClass
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class AdaptiveLayoutTest {
    @OptIn(ExperimentalMaterial3AdaptiveApi::class)
    private fun directive(widthDp: Float) = adaptiveDirective(WindowAdaptiveInfo(WindowSizeClass.compute(widthDp, 800f), Posture()))

    @OptIn(ExperimentalMaterial3AdaptiveApi::class)
    @Test
    fun adaptiveDirectiveSplitsTheFoldInnerScreenOnly() {
        assertEquals(2, directive(700f).maxHorizontalPartitions)
        assertEquals(24.dp, directive(700f).horizontalPartitionSpacerSize)
        assertEquals(1, directive(360f).maxHorizontalPartitions)
    }

    @Test
    fun layoutForBoundaries() {
        assertEquals(Layout.Compact, layoutFor(599))
        assertEquals(Layout.Medium, layoutFor(600))
        assertEquals(Layout.Medium, layoutFor(839))
        assertEquals(Layout.Expanded, layoutFor(840))
    }

    @Test
    fun foldInnerScreenGetsTwoPanesAndARail() {
        assertTrue(twoPane(700))
        assertEquals(NavKind.Rail, navKind(700))
    }

    @Test
    fun foldCoverScreenGetsOnePaneAndABottomBar() {
        assertFalse(twoPane(360))
        assertEquals(NavKind.BottomBar, navKind(360))
    }

    @Test
    fun twoPaneSwitchesExactlyAt600() {
        assertFalse(twoPane(599))
        assertTrue(twoPane(600))
    }

    @Test
    fun horizontalPartitionsSplitAt600() {
        assertEquals(1, horizontalPartitions(599))
        assertEquals(2, horizontalPartitions(600))
        assertEquals(2, horizontalPartitions(700))
    }
}
