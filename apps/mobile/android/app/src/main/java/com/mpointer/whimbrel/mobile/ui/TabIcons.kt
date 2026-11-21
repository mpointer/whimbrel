package com.mpointer.whimbrel.mobile.ui

import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.SolidColor
import androidx.compose.ui.graphics.vector.ImageVector
import androidx.compose.ui.graphics.vector.addPathNodes
import androidx.compose.runtime.Composable
import androidx.compose.ui.platform.LocalDensity
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp

// The two tab icons the core Material set lacks (the extended set is not a dependency): Material's
// "chat bubble" and "label" shapes, as on the iPhone's Ask and Topics tabs.

private fun icon(name: String, path: String): ImageVector =
    ImageVector.Builder(name, 24.dp, 24.dp, 24f, 24f).addPath(addPathNodes(path), fill = SolidColor(Color.Black)).build()

internal val AskTabIcon: ImageVector by lazy {
    icon("Ask", "M20,2H4c-1.1,0 -2,0.9 -2,2v18l4,-4h14c1.1,0 2,-0.9 2,-2V4c0,-1.1 -0.9,-2 -2,-2z")
}

internal val TopicsTabIcon: ImageVector by lazy {
    icon("Topics", "M17.63,5.84C17.27,5.33 16.67,5 16,5L5,5.01C3.9,5.01 3,5.9 3,7v10c0,1.1 0.9,1.99 2,1.99L16,19c0.67,0 1.27,-0.33 1.63,-0.84L22,12l-4.37,-6.16z")
}

/** A filled square: the usual mark for "stop". */
internal val StopIcon: ImageVector by lazy { icon("Stop", "M6,6h12v12H6z") }

/** A size for an icon that sits in a line of text: [base] dp at the default text size, growing with the text. */
@Composable
internal fun textIconSize(base: Int): Dp = with(LocalDensity.current) { base.sp.toDp() }
