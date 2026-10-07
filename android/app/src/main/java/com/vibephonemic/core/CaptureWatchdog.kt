package com.vibephonemic.core

/**
 * Detects a wedged capture source: ~1 s of exact digital zeros while unmuted, or repeated read
 * errors. Asks the capture loop to reopen it, at most [maxConsecutiveReopens] times in a row
 * (a frame with real signal resets the budget), then gives up.
 */
class CaptureWatchdog(
    private val zeroFramesLimit: Int = 100,   // 100 x 10 ms = 1 s
    private val errorLimit: Int = 10,
    private val maxConsecutiveReopens: Int = 5,
) {
    enum class Action { OK, REOPEN, GIVE_UP }

    private var zeroFrames = 0
    private var errors = 0
    private var gaveUp = false

    /** Reopens since the last frame that carried signal. */
    var consecutiveReopens = 0
        private set

    fun onReadError(): Action {
        zeroFrames = 0
        if (++errors < errorLimit) return Action.OK
        errors = 0
        return escalate()
    }

    /**
     * Call with the raw (pre-gain) samples of every successful read. [monitored] is false for
     * sources without a watchdog (test tone, silence) and while the user has muted.
     */
    fun onFrame(samples: ShortArray, count: Int, monitored: Boolean): Action {
        errors = 0
        if (!monitored) { zeroFrames = 0; return Action.OK }
        if (!isDigitalSilence(samples, count)) {
            zeroFrames = 0
            consecutiveReopens = 0
            gaveUp = false
            return Action.OK
        }
        if (gaveUp) return Action.OK
        if (++zeroFrames < zeroFramesLimit) return Action.OK
        zeroFrames = 0
        return escalate()
    }

    private fun escalate(): Action =
        if (consecutiveReopens >= maxConsecutiveReopens) {
            gaveUp = true
            Action.GIVE_UP
        } else {
            consecutiveReopens++
            Action.REOPEN
        }

    companion object {
        fun isDigitalSilence(samples: ShortArray, count: Int): Boolean {
            for (i in 0 until count) if (samples[i].toInt() != 0) return false
            return true
        }
    }
}
