package com.vibephonemic.core

import kotlin.math.log10
import kotlin.math.roundToInt
import kotlin.math.sqrt

object AudioMath {
    const val MIN_VOLUME = 0
    const val MAX_VOLUME = 200

    fun clampVolume(percent: Int): Int = percent.coerceIn(MIN_VOLUME, MAX_VOLUME)

    /** Applies gain (percent, 0..200) or mute in place, clipping to the s16 range. */
    fun applyGain(samples: ShortArray, count: Int, volumePercent: Int, muted: Boolean) {
        if (muted || volumePercent <= 0) {
            java.util.Arrays.fill(samples, 0, count, 0)
            return
        }
        val vol = clampVolume(volumePercent)
        if (vol == 100) return
        for (i in 0 until count) samples[i] = gainSample(samples[i], vol)
    }

    fun gainSample(sample: Short, volumePercent: Int): Short {
        val v = sample.toInt() * volumePercent / 100
        return v.coerceIn(Short.MIN_VALUE.toInt(), Short.MAX_VALUE.toInt()).toShort()
    }

    fun rms(samples: ShortArray, count: Int): Double {
        if (count <= 0) return 0.0
        var sum = 0.0
        for (i in 0 until count) { val s = samples[i].toDouble(); sum += s * s }
        return sqrt(sum / count)
    }

    /** Meter level 0..1 mapping RMS from -60 dBFS..0 dBFS. */
    fun meterLevel(samples: ShortArray, count: Int): Float {
        val r = rms(samples, count)
        if (r < 1.0) return 0f
        val db = 20.0 * log10(r / 32768.0)
        return ((db + 60.0) / 60.0).coerceIn(0.0, 1.0).toFloat()
    }

    fun percentLabel(volume: Int): String = "${clampVolume(volume)} %"

    fun sliderToVolume(value: Float): Int = clampVolume(value.roundToInt())
}
