package com.vibephonemic.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.selection.toggleable
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.Icon
import androidx.compose.material3.MaterialTheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.draw.clip
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.res.painterResource
import androidx.compose.ui.semantics.ProgressBarRangeInfo
import androidx.compose.ui.semantics.Role
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.progressBarRangeInfo
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.stateDescription
import androidx.compose.ui.unit.Dp
import androidx.compose.ui.unit.dp
import com.vibephonemic.R

/** Small round logo: gradient disc with a white mic. */
@Composable
fun Logo(size: Dp = 36.dp) {
    Box(
        Modifier.size(size).clip(CircleShape).background(BrandGradient),
        contentAlignment = Alignment.Center,
    ) {
        Icon(painterResource(R.drawable.ic_mic), contentDescription = null, tint = Color.White, modifier = Modifier.size(size * 0.6f))
    }
}

/** Big round mute toggle. Gradient when live, neutral gray when muted. */
@Composable
fun MuteButton(muted: Boolean, onToggle: () -> Unit, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    val bg = if (muted) Modifier.background(colors.surfaceContainerHighest) else Modifier.background(BrandGradient)
    Box(
        modifier
            .size(120.dp)
            .clip(CircleShape)
            .then(bg)
            .toggleable(value = muted, role = Role.Switch, onValueChange = { onToggle() })
            .semantics {
                contentDescription = "Mute microphone"
                stateDescription = if (muted) "Muted" else "Live"
            }
            .testTag("mute_button"),
        contentAlignment = Alignment.Center,
    ) {
        Icon(
            painterResource(if (muted) R.drawable.ic_mic_off else R.drawable.ic_mic),
            contentDescription = null,
            tint = if (muted) colors.onSurfaceVariant else Color.White,
            modifier = Modifier.size(58.dp),
        )
    }
}

/** Horizontal level meter (0..1) with the brand gradient as fill. */
@Composable
fun LevelMeter(level: Float, modifier: Modifier = Modifier) {
    val track = MaterialTheme.colorScheme.surfaceContainerHighest
    val clamped = level.coerceIn(0f, 1f)
    Canvas(
        modifier
            .fillMaxWidth()
            .height(12.dp)
            .semantics {
                contentDescription = "Input level"
                progressBarRangeInfo = ProgressBarRangeInfo(clamped, 0f..1f)
            }
            .testTag("level_meter"),
    ) {
        val r = CornerRadius(size.height / 2, size.height / 2)
        drawRoundRect(track, cornerRadius = r)
        if (clamped > 0f) {
            drawRoundRect(
                brush = brandHorizontalGradient(size.width),
                size = Size(maxOf(size.height, size.width * clamped), size.height),
                cornerRadius = r,
            )
        }
    }
}

/** Status dot: gradient when active, neutral otherwise. */
@Composable
fun StatusDot(active: Boolean) {
    Box(
        Modifier
            .padding(end = 10.dp)
            .size(12.dp)
            .clip(CircleShape)
            .then(if (active) Modifier.background(BrandGradient) else Modifier.background(MaterialTheme.colorScheme.outline)),
    )
}
