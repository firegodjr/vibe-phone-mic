package com.vibephonemic.ui

import android.graphics.Bitmap
import android.graphics.Canvas
import androidx.activity.ComponentActivity
import androidx.compose.ui.test.junit4.createAndroidComposeRule
import androidx.compose.ui.test.onNodeWithTag
import com.vibephonemic.LocalAddress
import com.vibephonemic.ServerState
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.annotation.Config
import org.robolectric.annotation.GraphicsMode
import java.io.File

/**
 * Renders the main screen with Robolectric native graphics, checks real pixels for theme colors
 * and writes PNGs to app/build/screenshots/ for visual review.
 */
@RunWith(RobolectricTestRunner::class)
@GraphicsMode(GraphicsMode.Mode.NATIVE)
@Config(qualifiers = "w411dp-h840dp-xxhdpi")
class ScreenshotTest {
    @get:Rule val rule = createAndroidComposeRule<ComponentActivity>()

    private val state = MainUiState(
        addresses = listOf(LocalAddress("Wi-Fi", "192.168.1.42"), LocalAddress("USB tethering", "192.168.42.129")),
        server = ServerState(running = true, clientAddress = "192.168.1.10", sampleRate = 48000, inputDescription = "Built-in microphone"),
        volume = 120,
    )

    private fun render(dark: Boolean, muted: Boolean = false): Bitmap {
        rule.setContent { VpmTheme(darkTheme = dark) { MainScreen(state.copy(muted = muted), 0.7f, {}, {}, {}, {}, {}) } }
        rule.waitForIdle()
        // Software-draw the window (captureToImage() waits for a hardware frame Robolectric never signals).
        val view = rule.activity.window.decorView
        val bmp = Bitmap.createBitmap(view.width, view.height, Bitmap.Config.ARGB_8888)
        view.draw(Canvas(bmp))
        val dir = File("build/screenshots").apply { mkdirs() }
        File(dir, "main-${if (dark) "dark" else "light"}${if (muted) "-muted" else ""}.png").outputStream().use {
            bmp.compress(Bitmap.CompressFormat.PNG, 100, it)
        }
        return bmp
    }

    private fun rgb(p: Int) = Triple((p shr 16) and 0xFF, (p shr 8) and 0xFF, p and 0xFF)

    @Test fun darkThemePixels() {
        val bmp = render(dark = true)
        // page background (top-left corner, outside cards) is dark gray #1C1C1C
        assertEquals(Triple(0x1C, 0x1C, 0x1C), rgb(bmp.getPixel(2, 2)))
        assertMuteButtonIsBrandColored(bmp)
    }

    @Test fun lightThemePixels() {
        val bmp = render(dark = false)
        assertEquals(Triple(0xFF, 0xFF, 0xFF), rgb(bmp.getPixel(2, 2)))
        assertMuteButtonIsBrandColored(bmp)
    }

    @Test fun mutedButtonIsNeutral() {
        val bmp = render(dark = false, muted = true)
        val (r, g, b) = rgb(centerPixelOfMute(bmp, offset = 0.3f))
        assertTrue("muted button should be gray, was $r,$g,$b", maxOf(r, g, b) - minOf(r, g, b) < 8)
    }

    private fun centerPixelOfMute(bmp: Bitmap, offset: Float): Int {
        val bounds = rule.onNodeWithTag("mute_button").fetchSemanticsNode().boundsInRoot
        // sample left of the icon, inside the circle
        val x = (bounds.left + bounds.width * offset).toInt()
        val y = bounds.center.y.toInt()
        return bmp.getPixel(x, y)
    }

    private fun assertMuteButtonIsBrandColored(bmp: Bitmap) {
        val (r, g, b) = rgb(centerPixelOfMute(bmp, offset = 0.2f))
        // somewhere on the #FF8A00 -> #E52E2E gradient: red high, blue low
        assertTrue("expected orange/red, was $r,$g,$b", r > 200 && b < 60 && g in 30..150)
    }
}
