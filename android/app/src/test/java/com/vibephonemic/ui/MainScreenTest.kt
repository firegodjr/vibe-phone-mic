package com.vibephonemic.ui

import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.setValue
import androidx.compose.ui.semantics.SemanticsActions
import androidx.compose.ui.semantics.SemanticsProperties
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsEnabled
import androidx.compose.ui.test.assertIsNotEnabled
import androidx.compose.ui.test.assertIsOff
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.assertTextEquals
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.onRoot
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.performSemanticsAction
import com.vibephonemic.LocalAddress
import com.vibephonemic.ServerState
import androidx.compose.material3.ColorScheme
import androidx.compose.material3.MaterialTheme
import androidx.compose.ui.graphics.Color
import androidx.compose.ui.graphics.luminance
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.robolectric.annotation.Config
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class MainScreenTest {
    @get:Rule val rule = createComposeRule()

    private val listening = ServerState(running = true, sampleRate = 48000, inputDescription = "Default microphone")

    private class Calls {
        var mute = 0; var startStop = 0; var grant = 0; var settings = 0
        val volumes = mutableListOf<Int>()
    }

    private fun show(state: MainUiState, level: Float = 0f, dark: Boolean = false): Calls {
        val calls = Calls()
        rule.setContent {
            VpmTheme(darkTheme = dark) {
                MainScreen(
                    state = state, level = level,
                    onToggleMute = { calls.mute++ },
                    onVolumeChange = { calls.volumes += it },
                    onStartStop = { calls.startStop++ },
                    onGrantPermission = { calls.grant++ },
                    onOpenSettings = { calls.settings++ },
                )
            }
        }
        return calls
    }

    @Test fun noNetworkShowsHintAndPort() {
        show(MainUiState(addresses = emptyList(), server = listening))
        rule.onNodeWithTag("no_network_text").assertIsDisplayed()
        rule.onAllNodesWithTag("ip_text").assertCountEquals(0)
        rule.onNodeWithTag("port_text").assertTextEquals("48777")
        rule.onNodeWithTag("usb_hint").assertTextEquals(
            "USB: run  adb forward tcp:48777 tcp:48777  on the computer, then connect to 127.0.0.1"
        )
    }

    @Test fun singleIp() {
        show(MainUiState(addresses = listOf(LocalAddress("Wi-Fi", "192.168.1.42")), server = listening))
        rule.onAllNodesWithTag("no_network_text").assertCountEquals(0)
        rule.onAllNodesWithTag("ip_text").assertCountEquals(1)
        rule.onNodeWithTag("ip_text").assertTextEquals("192.168.1.42").assertIsDisplayed()
        rule.onNodeWithText("Wi-Fi").assertIsDisplayed()
    }

    @Test fun multipleIps() {
        show(MainUiState(addresses = listOf(LocalAddress("Wi-Fi", "192.168.1.42"), LocalAddress("USB tethering", "192.168.42.129")), server = listening))
        val ips = rule.onAllNodesWithTag("ip_text")
        ips.assertCountEquals(2)
        ips[0].assertTextEquals("192.168.1.42")
        ips[1].assertTextEquals("192.168.42.129")
        rule.onNodeWithText("USB tethering").assertIsDisplayed()
    }

    @Test fun waitingForClient() {
        show(MainUiState(server = listening))
        rule.onNodeWithTag("connection_status").assertTextEquals("Waiting for computer…")
        rule.onNodeWithTag("audio_info").assertTextEquals("48000 Hz mono · Default microphone")
        rule.onNodeWithTag("server_toggle").assertTextEquals("Stop")
    }

    @Test fun connectedClient() {
        show(MainUiState(server = listening.copy(clientAddress = "192.168.1.10")))
        rule.onNodeWithTag("connection_status").assertTextEquals("Connected to 192.168.1.10")
    }

    @Test fun stoppedServerOffersStart() {
        val calls = show(MainUiState(server = ServerState(running = false)))
        rule.onNodeWithTag("connection_status").assertTextEquals("Server stopped")
        rule.onAllNodesWithTag("audio_info").assertCountEquals(0)
        rule.onNodeWithTag("server_toggle").assertTextEquals("Start").assertIsEnabled().performClick()
        assertEquals(1, calls.startStop)
    }

    @Test fun errorIsShown() {
        show(MainUiState(server = listening.copy(error = "Microphone error: busy")))
        rule.onNodeWithTag("error_text").assertTextEquals("Microphone error: busy")
    }

    @Test fun permissionDenied() {
        val calls = show(MainUiState(permissionGranted = false, server = ServerState()))
        rule.onNodeWithTag("permission_card").assertIsDisplayed()
        rule.onNodeWithTag("grant_permission").performClick()
        assertEquals(1, calls.grant)
        rule.onNodeWithTag("server_toggle").performScrollTo().assertIsNotEnabled()
    }

    @Test fun permissionGranted() {
        val calls = show(MainUiState(permissionGranted = true, addresses = listOf(LocalAddress("Wi-Fi", "192.168.1.42")), server = listening))
        rule.onAllNodesWithTag("permission_card").assertCountEquals(0)
        rule.onAllNodesWithTag("grant_permission").assertCountEquals(0)
        // the normal content is there and usable
        rule.onNodeWithTag("ip_text").assertTextEquals("192.168.1.42").assertIsDisplayed()
        rule.onNodeWithTag("connection_status").assertTextEquals("Waiting for computer…")
        rule.onNodeWithTag("server_toggle").assertTextEquals("Stop").assertIsEnabled().performClick()
        rule.onNodeWithTag("mute_button").performScrollTo().assertIsEnabled().performClick()
        assertEquals(1, calls.startStop)
        assertEquals(1, calls.mute)
    }

    private fun fontSizeOf(node: androidx.compose.ui.test.SemanticsNodeInteraction): androidx.compose.ui.unit.TextUnit {
        val results = mutableListOf<androidx.compose.ui.text.TextLayoutResult>()
        node.performSemanticsAction(SemanticsActions.GetTextLayoutResult) { it(results) }
        return results.first().layoutInput.style.fontSize
    }

    @Test fun singleIpUsesLargeTextMultipleIpsSmaller() {
        var addrs by mutableStateOf(listOf(LocalAddress("Wi-Fi", "192.168.1.42")))
        rule.setContent { VpmTheme { MainScreen(MainUiState(addresses = addrs, server = listening), 0f, {}, {}, {}, {}, {}) } }
        assertEquals(IP_SIZE_SINGLE, fontSizeOf(rule.onNodeWithTag("ip_text")))
        addrs = listOf(LocalAddress("Wi-Fi", "10.0.2.16"), LocalAddress("Ethernet", "10.0.2.15"))
        rule.waitForIdle()
        assertEquals(IP_SIZE_MANY, fontSizeOf(rule.onAllNodesWithTag("ip_text")[0]))
        assertEquals(IP_SIZE_MANY, fontSizeOf(rule.onAllNodesWithTag("ip_text")[1]))
    }

    @Test fun liveMuteButton() {
        val calls = show(MainUiState(muted = false, server = listening))
        rule.onNodeWithTag("mute_button").performScrollTo().assertIsOff()
        rule.onNodeWithTag("mute_state").assertTextEquals("Live — tap to mute")
        rule.onNodeWithTag("mute_button").performClick()
        assertEquals(1, calls.mute)
    }

    @Test fun mutedMuteButton() {
        show(MainUiState(muted = true, server = listening))
        rule.onNodeWithTag("mute_button").performScrollTo().assertIsOn()
        rule.onNodeWithTag("mute_state").assertTextEquals("Muted — tap to unmute")
    }

    @Test fun muteToggleRoundTripWithState() {
        var muted by mutableStateOf(false)
        rule.setContent {
            VpmTheme {
                MainScreen(MainUiState(muted = muted, server = listening), 0f, { muted = !muted }, {}, {}, {}, {})
            }
        }
        rule.onNodeWithTag("mute_button").performScrollTo().performClick().assertIsOn()
        rule.onNodeWithTag("mute_button").performClick().assertIsOff()
    }

    @Test fun volumeButtonsAndSlider() {
        val calls = show(MainUiState(volume = 100, server = listening))
        rule.onNodeWithTag("volume_text").performScrollTo().assertTextEquals("100 %")
        rule.onNodeWithTag("volume_up").performScrollTo().performClick()
        rule.onNodeWithTag("volume_down").performClick()
        rule.onNodeWithTag("volume_slider").performSemanticsAction(SemanticsActions.SetProgress) { it(150f) }
        assertEquals(listOf(110, 90, 150), calls.volumes)
        val range = rule.onNodeWithTag("volume_slider").fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo]
        assertEquals(100f, range.current, 0f)
        assertEquals(0f, range.range.start, 0f)
        assertEquals(200f, range.range.endInclusive, 0f)
    }

    @Test fun volumeLimitsAtZeroAndTwoHundred() {
        val atMax = show(MainUiState(volume = 200, server = listening))
        rule.onNodeWithTag("volume_text").performScrollTo().assertTextEquals("200 %")
        rule.onNodeWithTag("volume_up").performScrollTo().performClick()
        assertEquals(listOf(200), atMax.volumes)
    }

    @Test fun volumeAtZero() {
        val atMin = show(MainUiState(volume = 0, server = listening))
        rule.onNodeWithTag("volume_text").performScrollTo().assertTextEquals("0 %")
        rule.onNodeWithTag("volume_down").performScrollTo().performClick()
        assertEquals(listOf(0), atMin.volumes)
    }

    @Test fun levelMeterReflectsLevel() {
        show(MainUiState(server = listening), level = 0.6f)
        val info = rule.onNodeWithTag("level_meter").performScrollTo().fetchSemanticsNode().config[SemanticsProperties.ProgressBarRangeInfo]
        assertEquals(0.6f, info.current, 0.001f)
    }

    @Test fun settingsButton() {
        val calls = show(MainUiState(server = listening))
        rule.onNodeWithTag("settings_button").performClick()
        assertEquals(1, calls.settings)
    }

    @Test fun darkThemeUsesDarkGrayNeutralScheme() {
        var scheme: ColorScheme? = null
        rule.setContent { VpmTheme(darkTheme = true) { scheme = MaterialTheme.colorScheme; MainScreen(MainUiState(server = listening), 0f, {}, {}, {}, {}, {}) } }
        rule.waitForIdle()
        val s = scheme!!
        assertEquals(Color(0xFF1C1C1C), s.background)
        assertEquals(Color(0xFF1C1C1C), s.surface)
        assertTrue("dark text on dark bg? ${s.onSurface}", s.onSurface.luminance() > 0.8f)
        assertNoAccentColors(s)
    }

    @Test fun lightThemeUsesWhiteNeutralScheme() {
        var scheme: ColorScheme? = null
        rule.setContent { VpmTheme(darkTheme = false) { scheme = MaterialTheme.colorScheme } }
        rule.waitForIdle()
        val s = scheme!!
        assertEquals(Color.White, s.background)
        assertEquals(Color.White, s.surface)
        assertTrue(s.onSurface.luminance() < 0.05f)
        assertNoAccentColors(s)
    }

    /** Every Material slot except the error colors must be a neutral gray (no purple/blue leaks). */
    private fun assertNoAccentColors(s: ColorScheme) {
        val slots = mapOf(
            "primary" to s.primary, "onPrimary" to s.onPrimary, "primaryContainer" to s.primaryContainer,
            "onPrimaryContainer" to s.onPrimaryContainer, "inversePrimary" to s.inversePrimary,
            "secondary" to s.secondary, "onSecondary" to s.onSecondary, "secondaryContainer" to s.secondaryContainer,
            "onSecondaryContainer" to s.onSecondaryContainer, "tertiary" to s.tertiary, "onTertiary" to s.onTertiary,
            "tertiaryContainer" to s.tertiaryContainer, "onTertiaryContainer" to s.onTertiaryContainer,
            "background" to s.background, "onBackground" to s.onBackground, "surface" to s.surface,
            "onSurface" to s.onSurface, "surfaceVariant" to s.surfaceVariant, "onSurfaceVariant" to s.onSurfaceVariant,
            "inverseSurface" to s.inverseSurface, "inverseOnSurface" to s.inverseOnSurface, "outline" to s.outline,
            "outlineVariant" to s.outlineVariant, "surfaceBright" to s.surfaceBright, "surfaceDim" to s.surfaceDim,
            "surfaceContainer" to s.surfaceContainer, "surfaceContainerHigh" to s.surfaceContainerHigh,
            "surfaceContainerHighest" to s.surfaceContainerHighest, "surfaceContainerLow" to s.surfaceContainerLow,
            "surfaceContainerLowest" to s.surfaceContainerLowest,
        )
        for ((name, c) in slots) {
            val spread = maxOf(c.red, c.green, c.blue) - minOf(c.red, c.green, c.blue)
            assertTrue("$name is not neutral: $c", spread < 0.01f)
        }
        assertEquals(0f, s.surfaceTint.alpha, 0f)
    }

    @Test
    @Config(qualifiers = "w411dp-h840dp") // 1080x2400 @420dpi minus status/nav bars
    fun everythingFitsOnAPhoneScreenWithTwoIps() {
        show(MainUiState(addresses = listOf(LocalAddress("Wi-Fi", "10.0.2.16"), LocalAddress("Ethernet", "10.0.2.15")), server = listening.copy(clientAddress = "127.0.0.1")))
        // no scrolling: these must all be on screen as laid out
        for (tag in listOf("server_toggle", "connection_status", "port_text", "mute_button", "level_meter", "volume_slider", "volume_up")) {
            try { rule.onNodeWithTag(tag).assertIsDisplayed() } catch (e: AssertionError) {
                val n = rule.onNodeWithTag(tag, useUnmergedTree = true).fetchSemanticsNode()
                val b = "${n.boundsInRoot} size=${n.size} placed=${n.layoutInfo.isPlaced} parent=${n.parent?.boundsInRoot} up=${rule.onNodeWithTag("volume_up").fetchSemanticsNode().boundsInRoot}"
                val root = rule.onRoot().fetchSemanticsNode().boundsInRoot
                throw AssertionError("$tag is below the fold: $b root=$root", e)
            }
        }
        rule.onAllNodesWithTag("ip_text")[1].assertIsDisplayed()
    }
}
