package com.vibephonemic.ui

import androidx.compose.foundation.isSystemInDarkTheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.Typography
import androidx.compose.material3.darkColorScheme
import androidx.compose.material3.lightColorScheme
import androidx.compose.runtime.Composable
import androidx.compose.ui.graphics.Brush
import androidx.compose.ui.graphics.Color

/** The single pop color of the app: orange → red. */
val BrandOrange = Color(0xFFFF8A00)
val BrandRed = Color(0xFFE52E2E)
val BrandGradient = Brush.linearGradient(listOf(BrandOrange, BrandRed))
fun brandHorizontalGradient(width: Float) =
    Brush.horizontalGradient(listOf(BrandOrange, BrandRed), startX = 0f, endX = width)

// Neutral palettes only. Every Material slot is set so no default purple/blue can leak in;
// dynamic color is deliberately not used.
private val Light = lightColorScheme(
    primary = Color(0xFF262626),
    onPrimary = Color.White,
    primaryContainer = Color(0xFFEDEDED),
    onPrimaryContainer = Color(0xFF1F1F1F),
    inversePrimary = Color(0xFFE6E6E6),
    secondary = Color(0xFF4A4A4A),
    onSecondary = Color.White,
    secondaryContainer = Color(0xFFEDEDED),
    onSecondaryContainer = Color(0xFF1F1F1F),
    tertiary = Color(0xFF4A4A4A),
    onTertiary = Color.White,
    tertiaryContainer = Color(0xFFEDEDED),
    onTertiaryContainer = Color(0xFF1F1F1F),
    background = Color.White,
    onBackground = Color(0xFF1F1F1F),
    surface = Color.White,
    onSurface = Color(0xFF1F1F1F),
    surfaceVariant = Color(0xFFF2F2F2),
    onSurfaceVariant = Color(0xFF5E5E5E),
    surfaceTint = Color.Transparent,
    inverseSurface = Color(0xFF2B2B2B),
    inverseOnSurface = Color(0xFFF2F2F2),
    error = BrandRed,
    onError = Color.White,
    errorContainer = Color(0xFFF7E3E3),
    onErrorContainer = Color(0xFF5C1212),
    outline = Color(0xFFC9C9C9),
    outlineVariant = Color(0xFFE3E3E3),
    scrim = Color.Black,
    surfaceBright = Color.White,
    surfaceDim = Color(0xFFE6E6E6),
    surfaceContainerLowest = Color.White,
    surfaceContainerLow = Color(0xFFF8F8F8),
    surfaceContainer = Color(0xFFF4F4F4),
    surfaceContainerHigh = Color(0xFFEEEEEE),
    surfaceContainerHighest = Color(0xFFE8E8E8),
)

private val Dark = darkColorScheme(
    primary = Color(0xFFEDEDED),
    onPrimary = Color(0xFF1C1C1C),
    primaryContainer = Color(0xFF333333),
    onPrimaryContainer = Color(0xFFEDEDED),
    inversePrimary = Color(0xFF262626),
    secondary = Color(0xFFC4C4C4),
    onSecondary = Color(0xFF1C1C1C),
    secondaryContainer = Color(0xFF333333),
    onSecondaryContainer = Color(0xFFEDEDED),
    tertiary = Color(0xFFC4C4C4),
    onTertiary = Color(0xFF1C1C1C),
    tertiaryContainer = Color(0xFF333333),
    onTertiaryContainer = Color(0xFFEDEDED),
    background = Color(0xFF1C1C1C),
    onBackground = Color(0xFFF0F0F0),
    surface = Color(0xFF1C1C1C),
    onSurface = Color(0xFFF0F0F0),
    surfaceVariant = Color(0xFF2A2A2A),
    onSurfaceVariant = Color(0xFFABABAB),
    surfaceTint = Color.Transparent,
    inverseSurface = Color(0xFFEDEDED),
    inverseOnSurface = Color(0xFF1C1C1C),
    error = Color(0xFFFF6B5E),
    onError = Color(0xFF1C1C1C),
    errorContainer = Color(0xFF4A1E1E),
    onErrorContainer = Color(0xFFFFDAD6),
    outline = Color(0xFF555555),
    outlineVariant = Color(0xFF3A3A3A),
    scrim = Color.Black,
    surfaceBright = Color(0xFF383838),
    surfaceDim = Color(0xFF1C1C1C),
    surfaceContainerLowest = Color(0xFF171717),
    surfaceContainerLow = Color(0xFF212121),
    surfaceContainer = Color(0xFF252525),
    surfaceContainerHigh = Color(0xFF2C2C2C),
    surfaceContainerHighest = Color(0xFF333333),
)

@Composable
fun VpmTheme(darkTheme: Boolean = isSystemInDarkTheme(), content: @Composable () -> Unit) {
    MaterialTheme(colorScheme = if (darkTheme) Dark else Light, typography = Typography(), content = content)
}
