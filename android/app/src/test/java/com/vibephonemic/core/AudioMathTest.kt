package com.vibephonemic.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class AudioMathTest {
    private fun gained(vol: Int, muted: Boolean, vararg s: Short): ShortArray =
        s.copyOf().also { AudioMath.applyGain(it, it.size, vol, muted) }

    @Test fun unityGainIsIdentity() {
        assertArrayEquals(shortArrayOf(100, -100, 32767, -32768), gained(100, false, 100, -100, 32767, -32768))
    }

    @Test fun halfAndDouble() {
        assertArrayEquals(shortArrayOf(50, -50, 0), gained(50, false, 100, -100, 0))
        assertArrayEquals(shortArrayOf(200, -200, 30000), gained(200, false, 100, -100, 15000))
    }

    @Test fun clipsInsteadOfWrapping() {
        assertArrayEquals(shortArrayOf(32767, -32768, 32767, -32768), gained(200, false, 20000, -20000, 32767, -32768))
        assertArrayEquals(shortArrayOf(32767), gained(150, false, 30000))
    }

    @Test fun muteAndZeroVolumeProduceSilence() {
        assertArrayEquals(ShortArray(3), gained(100, true, 1, -5, 30000))
        assertArrayEquals(ShortArray(3), gained(200, true, 1, -5, 30000))
        assertArrayEquals(ShortArray(3), gained(0, false, 1, -5, 30000))
    }

    @Test fun onlyCountSamplesAreProcessed() {
        val s = shortArrayOf(10, 10, 10)
        AudioMath.applyGain(s, 2, 100, true)
        assertArrayEquals(shortArrayOf(0, 0, 10), s)
    }

    @Test fun volumeClamped() {
        assertEquals(0, AudioMath.clampVolume(-5))
        assertEquals(200, AudioMath.clampVolume(500))
        assertEquals(137, AudioMath.sliderToVolume(136.6f))
        assertArrayEquals(shortArrayOf(200), gained(900, false, 100))
    }

    @Test fun meterLevel() {
        assertEquals(0f, AudioMath.meterLevel(ShortArray(480), 480), 0f)
        val full = ShortArray(480) { if (it % 2 == 0) 32767 else -32767 }
        assertEquals(1f, AudioMath.meterLevel(full, 480), 0.01f)
        val quiet = ShortArray(480) { 33 } // ~ -60 dBFS
        assertTrue(AudioMath.meterLevel(quiet, 480) < 0.05f)
    }
}
