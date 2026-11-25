package com.vibephonemic.core

/**
 * Temporarily forces the outgoing stream to digital silence while the app itself makes a sound, so the phone's
 * mic never streams the app's own button clicks.
 *
 * [hold] is called *before* a sound is played: the capture loop zeroes every frame it processes while [isClosed].
 * Frames processed after that point were captured no earlier than slightly before the call, so no part of the
 * click gets through. Overlapping holds extend each other (the gate opens at the latest deadline). This is
 * separate from the user's mute: STATUS keeps reporting the real mute state.
 */
class SilenceGate(private val clockNanos: () -> Long = System::nanoTime) {
    private val lock = Any()
    private var untilNanos = 0L
    @Volatile private var active = false

    fun hold(millis: Long) {
        val end = clockNanos() + millis * 1_000_000
        synchronized(lock) {
            if (!active || end - untilNanos > 0) untilNanos = end
            active = true
        }
    }

    val isClosed: Boolean
        get() {
            if (!active) return false
            synchronized(lock) {
                if (active && clockNanos() - untilNanos >= 0) active = false
                return active
            }
        }
}
