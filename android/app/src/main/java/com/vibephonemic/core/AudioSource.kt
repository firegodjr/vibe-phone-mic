package com.vibephonemic.core

import kotlin.math.PI
import kotlin.math.sin

/** A blocking source of mono PCM16 audio. */
interface AudioSource {
    /** Sample rate in Hz; valid after [start] returned. */
    val sampleRate: Int

    /** Human-readable name reported in STATUS.input. */
    val description: String

    /** Opens the source. Throws on failure. */
    fun start()

    /** Blocks until up to [count] samples are read into [buf]; returns the number read, or <0 on error. */
    fun read(buf: ShortArray, count: Int): Int

    fun stop()

    /**
     * True for real capture devices that can wedge (deliver endless digital zeros or errors), so
     * the capture loop may reopen them. Generated sources return false.
     */
    val watchdogEnabled: Boolean get() = false
}

/** Paces [read] calls to real time so generated sources behave like a microphone. */
private class RealTimePacer(private val sampleRate: Int) {
    private var startNanos = 0L
    private var produced = 0L

    fun reset() { startNanos = System.nanoTime(); produced = 0 }

    fun pace(samples: Int) {
        produced += samples
        val due = startNanos + produced * 1_000_000_000L / sampleRate
        val wait = due - System.nanoTime()
        if (wait > 0) Thread.sleep(wait / 1_000_000, (wait % 1_000_000).toInt())
    }
}

/** Generates a sine tone (used by tests and the debug-only test-tone hook). */
class SineAudioSource(
    val frequencyHz: Double = 440.0,
    override val sampleRate: Int = 48000,
    private val amplitude: Double = 0.5,
    private val realTime: Boolean = true,
) : AudioSource {
    override val description: String = "Test tone ${frequencyHz.toInt()} Hz"
    private var phase = 0.0
    private val pacer = RealTimePacer(sampleRate)

    override fun start() { phase = 0.0; pacer.reset() }

    override fun read(buf: ShortArray, count: Int): Int {
        val step = 2 * PI * frequencyHz / sampleRate
        for (i in 0 until count) {
            buf[i] = (sin(phase) * amplitude * Short.MAX_VALUE).toInt().toShort()
            phase += step
            if (phase > 2 * PI) phase -= 2 * PI
        }
        if (realTime) pacer.pace(count)
        return count
    }

    override fun stop() {}
}

/** Silent fallback used when the microphone can't be opened. */
class SilenceAudioSource(
    override val sampleRate: Int = 48000,
    override val description: String = "Microphone unavailable",
) : AudioSource {
    private val pacer = RealTimePacer(sampleRate)
    override fun start() { pacer.reset() }
    override fun read(buf: ShortArray, count: Int): Int {
        java.util.Arrays.fill(buf, 0, count, 0)
        pacer.pace(count)
        return count
    }
    override fun stop() {}
}
