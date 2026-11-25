package com.vibephonemic.ui

import androidx.compose.runtime.CompositionLocalProvider
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.test.core.app.ApplicationProvider
import com.vibephonemic.LocalAddress
import com.vibephonemic.ServerState
import com.vibephonemic.core.SilenceGate
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class ButtonSoundsTest {
    @get:Rule val rule = createComposeRule()

    private class Recorder : SoundPlayer {
        val played = mutableListOf<Pair<ButtonSound, Boolean>>()
        override fun play(sound: ButtonSound, press: Boolean) {
            played += sound to press
        }
    }

    private fun showMain(rec: Recorder) {
        rule.setContent {
            VpmTheme(darkTheme = false) {
                CompositionLocalProvider(LocalSoundPlayer provides rec) {
                    MainScreen(
                        state = MainUiState(
                            addresses = listOf(LocalAddress("wlan0", "192.168.1.5")),
                            server = ServerState(running = true, sampleRate = 48000),
                        ),
                        level = 0f,
                        onToggleMute = {}, onVolumeChange = {}, onStartStop = {},
                        onGrantPermission = {}, onOpenSettings = {},
                    )
                }
            }
        }
    }

    private fun pressAndRelease(s: ButtonSound) = listOf(s to true, s to false)

    @Test fun eachButtonPlaysItsSoundOnPressAndRelease() {
        val rec = Recorder()
        showMain(rec)
        val expected = mapOf(
            "mute_button" to ButtonSound.HaloSpace,
            "server_toggle" to ButtonSound.KvmSwitch,
            "settings_button" to ButtonSound.CarsonMicroflip,
            "volume_up" to ButtonSound.CarsonMicroflip,
            "volume_down" to ButtonSound.CarsonMicroflip,
        )
        for ((tag, sound) in expected) {
            rule.onNodeWithTag(tag).performScrollTo()
            rec.played.clear()
            rule.onNodeWithTag(tag).performClick()
            rule.waitForIdle()
            assertEquals(tag, pressAndRelease(sound), rec.played)
        }
    }

    @Test fun settingsControlsPlayTinySound() {
        val rec = Recorder()
        rule.setContent {
            VpmTheme(darkTheme = false) {
                CompositionLocalProvider(LocalSoundPlayer provides rec) {
                    SettingsScreen(options = emptyList(), selectedId = 0, versionName = "1", onSelect = {}, onBack = {})
                }
            }
        }
        for (tag in listOf("back_button", "input_option_default")) {
            rec.played.clear()
            rule.onNodeWithTag(tag).performClick()
            rule.waitForIdle()
            assertEquals(tag, pressAndRelease(ButtonSound.CarsonMicroflip), rec.played)
        }
    }

    @Test fun onlyMuteButtonPlaysWhileLiveAndOnlyItClosesTheGate() {
        assertTrue(ButtonSound.HaloSpace.gated)
        assertEquals(listOf(ButtonSound.HaloSpace), ButtonSound.entries.filter { it.gated })
        for (s in ButtonSound.entries) {
            assertTrue(ButtonSounds.shouldPlay(s, muted = true))
            assertEquals(s.gated, ButtonSounds.shouldPlay(s, muted = false))
        }
        var muted = false
        val gate = SilenceGate()
        val sounds = ButtonSounds(ApplicationProvider.getApplicationContext(), gate) { muted }
        // Live: other buttons are skipped and leave the stream alone.
        sounds.play(ButtonSound.KvmSwitch, press = true)
        sounds.play(ButtonSound.CarsonMicroflip, press = false)
        assertFalse(gate.isClosed)
        // Muted: other buttons play, but the gate is unnecessary and stays open.
        muted = true
        sounds.play(ButtonSound.KvmSwitch, press = true)
        assertFalse(gate.isClosed)
        // Mute button: closes the gate before playing, in either state.
        sounds.play(ButtonSound.HaloSpace, press = true)
        assertTrue(gate.isClosed)
        sounds.release()
    }

    @Test fun soundPoolPlayerLoadsAllClipsAndPlays() {
        val sounds = ButtonSounds(ApplicationProvider.getApplicationContext(), isMuted = { true })
        for (s in ButtonSound.entries) {
            assertEquals(s.downs.size, s.ups.size)
            repeat(3) {
                sounds.play(s, press = true)
                sounds.play(s, press = false)
            }
        }
        sounds.release()
    }
}
