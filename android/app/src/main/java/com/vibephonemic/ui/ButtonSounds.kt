package com.vibephonemic.ui

import android.content.Context
import android.media.AudioAttributes
import android.media.SoundPool
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.interaction.PressInteraction
import androidx.compose.runtime.Composable
import androidx.compose.runtime.LaunchedEffect
import androidx.compose.runtime.remember
import androidx.compose.runtime.staticCompositionLocalOf
import com.vibephonemic.R
import com.vibephonemic.core.SilenceGate
import com.vibephonemic.core.VariantPicker

/**
 * UI click sounds from github.com/joshwcomeau/button-sounds (MIT, see sounds/). Every sound is several recordings
 * of the same switch; a press plays a random "down" sample and the release a random "up" sample, so repeated taps
 * never sound identical. Clips are cut by sounds/extract.py into res/raw.
 *
 * [volume] is applied on top of the system volume for touch/sonification sounds (25–50 % recommended upstream).
 *
 * The phone's mic is live, so clicks must never reach the stream. A [gated] sound (only the mute toggle) always
 * plays, behind the stream's silence gate. Every other sound plays only while the mic is muted.
 */
enum class ButtonSound(val downs: IntArray, val ups: IntArray, val volume: Float, val gated: Boolean = false) {
    /** Small buttons: server Start/Stop, Grant access. */
    KvmSwitch(
        intArrayOf(R.raw.sfx_kvm_switch_down1, R.raw.sfx_kvm_switch_down2, R.raw.sfx_kvm_switch_down3, R.raw.sfx_kvm_switch_down4, R.raw.sfx_kvm_switch_down5),
        intArrayOf(R.raw.sfx_kvm_switch_up1, R.raw.sfx_kvm_switch_up2, R.raw.sfx_kvm_switch_up3, R.raw.sfx_kvm_switch_up4, R.raw.sfx_kvm_switch_up5),
        volume = 0.9f, // the recording itself is quiet (peaks around -17 dBFS)
    ),

    /** Tiny buttons: settings gear, back, volume ±, input rows. Kept quiet. */
    CarsonMicroflip(
        intArrayOf(R.raw.sfx_carson_microflip_down1, R.raw.sfx_carson_microflip_down2, R.raw.sfx_carson_microflip_down3, R.raw.sfx_carson_microflip_down4, R.raw.sfx_carson_microflip_down5),
        intArrayOf(R.raw.sfx_carson_microflip_up1, R.raw.sfx_carson_microflip_up2, R.raw.sfx_carson_microflip_up3, R.raw.sfx_carson_microflip_up4, R.raw.sfx_carson_microflip_up5),
        volume = 0.25f,
    ),

    /** The big mute toggle. */
    HaloSpace(
        intArrayOf(R.raw.sfx_halo_space_down1, R.raw.sfx_halo_space_down2, R.raw.sfx_halo_space_down3, R.raw.sfx_halo_space_down4, R.raw.sfx_halo_space_down5, R.raw.sfx_halo_space_down6, R.raw.sfx_halo_space_down7),
        intArrayOf(R.raw.sfx_halo_space_up1, R.raw.sfx_halo_space_up2, R.raw.sfx_halo_space_up3, R.raw.sfx_halo_space_up4, R.raw.sfx_halo_space_up5, R.raw.sfx_halo_space_up6, R.raw.sfx_halo_space_up7),
        volume = 0.45f,
        gated = true,
    ),
}

/** Plays [ButtonSound]s. [None] is silent (default for previews and tests that don't provide one). */
interface SoundPlayer {
    /** [press] = true for the "down" sample (finger down), false for the "up" sample (finger up). */
    fun play(sound: ButtonSound, press: Boolean)

    object None : SoundPlayer {
        override fun play(sound: ButtonSound, press: Boolean) = Unit
    }
}

/**
 * SoundPool-backed player. Create once per Activity and [release] it in onDestroy.
 *
 * Nothing it plays is ever streamed:
 * - A [ButtonSound.gated] sound (the mute toggle) first closes [gate] for [GATE_HOLD_MS], so the stream is silent
 *   from just before the click until it has died away (clip ≤ ~100 ms + speaker output latency + capture latency
 *   + room tail). The stream goes silent on press, and on unmute it stays silent until the release click has
 *   played. The UI still updates immediately.
 * - Every other sound plays only while [isMuted], and is skipped while the mic is live.
 */
class ButtonSounds(
    context: Context,
    private val gate: SilenceGate? = null,
    private val isMuted: () -> Boolean = { false },
) : SoundPlayer {
    companion object {
        const val GATE_HOLD_MS = 500L

        /** Whether [sound] may play right now given the mic's mute state. */
        fun shouldPlay(sound: ButtonSound, muted: Boolean): Boolean = sound.gated || muted
    }

    private val pool = SoundPool.Builder()
        .setMaxStreams(6)
        .setAudioAttributes(
            AudioAttributes.Builder()
                .setUsage(AudioAttributes.USAGE_ASSISTANCE_SONIFICATION)
                .setContentType(AudioAttributes.CONTENT_TYPE_SONIFICATION)
                .build(),
        )
        .build()

    private class Bank(val ids: List<Int>) {
        val picker = VariantPicker(ids.size)
    }

    // Loading is asynchronous; a play() before a clip has loaded is silently dropped by SoundPool.
    private val banks: Map<Pair<ButtonSound, Boolean>, Bank> = ButtonSound.entries.flatMap { s ->
        listOf(
            (s to true) to Bank(s.downs.map { pool.load(context, it, 1) }),
            (s to false) to Bank(s.ups.map { pool.load(context, it, 1) }),
        )
    }.toMap()

    override fun play(sound: ButtonSound, press: Boolean) {
        if (!shouldPlay(sound, isMuted())) return
        val bank = banks.getValue(sound to press)
        if (sound.gated) gate?.hold(GATE_HOLD_MS)
        pool.play(bank.ids[bank.picker.next()], sound.volume, sound.volume, 1, 0, 1f)
    }

    fun release() = pool.release()
}

val LocalSoundPlayer = staticCompositionLocalOf<SoundPlayer> { SoundPlayer.None }

/**
 * An interaction source for a clickable that plays [sound]: the down sample on press, the up sample on release
 * (or cancel). Pass it as the component's `interactionSource`.
 */
@Composable
fun rememberSoundInteraction(sound: ButtonSound): MutableInteractionSource {
    val source = remember { MutableInteractionSource() }
    val player = LocalSoundPlayer.current
    LaunchedEffect(source, player, sound) {
        source.interactions.collect {
            when (it) {
                is PressInteraction.Press -> player.play(sound, press = true)
                is PressInteraction.Release, is PressInteraction.Cancel -> player.play(sound, press = false)
            }
        }
    }
    return source
}
