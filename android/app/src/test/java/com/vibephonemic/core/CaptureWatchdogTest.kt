package com.vibephonemic.core

import com.vibephonemic.core.CaptureWatchdog.Action
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class CaptureWatchdogTest {
    private val zeros = ShortArray(480)
    private val signal = ShortArray(480) { if (it % 2 == 0) 100 else -100 }

    private fun CaptureWatchdog.feed(frames: Int, buf: ShortArray, monitored: Boolean = true): List<Action> =
        (1..frames).map { onFrame(buf, buf.size, monitored) }

    @Test fun oneSecondOfZerosRequestsReopen() {
        val w = CaptureWatchdog(zeroFramesLimit = 100)
        val actions = w.feed(100, zeros)
        assertTrue(actions.dropLast(1).all { it == Action.OK })
        assertEquals(Action.REOPEN, actions.last())
        assertEquals(1, w.consecutiveReopens)
    }

    @Test fun signalResetsZeroCounter() {
        val w = CaptureWatchdog(zeroFramesLimit = 100)
        w.feed(99, zeros)
        w.feed(1, signal)
        assertTrue(w.feed(99, zeros).all { it == Action.OK })
    }

    @Test fun mutedOrUnmonitoredSilenceIsIgnored() {
        val w = CaptureWatchdog(zeroFramesLimit = 10)
        assertTrue(w.feed(500, zeros, monitored = false).all { it == Action.OK })
    }

    @Test fun nearSilenceIsNotDigitalSilence() {
        val w = CaptureWatchdog(zeroFramesLimit = 10)
        val hiss = ShortArray(480).also { it[123] = 1 }
        assertTrue(w.feed(100, hiss).all { it == Action.OK })
        assertTrue(CaptureWatchdog.isDigitalSilence(zeros, 480))
        assertFalse(CaptureWatchdog.isDigitalSilence(hiss, 480))
    }

    @Test fun boundedReopensThenGiveUpUntilSignalReturns() {
        val w = CaptureWatchdog(zeroFramesLimit = 10, maxConsecutiveReopens = 3)
        val actions = w.feed(10 * 10, zeros)
        assertEquals(listOf(Action.REOPEN, Action.REOPEN, Action.REOPEN, Action.GIVE_UP), actions.filter { it != Action.OK })
        // after giving up: silent, no further requests
        assertTrue(w.feed(1000, zeros).all { it == Action.OK })
        // signal comes back: budget restored
        w.feed(1, signal)
        assertEquals(0, w.consecutiveReopens)
        assertEquals(Action.REOPEN, w.feed(10, zeros).last())
    }

    @Test fun readErrorsRequestReopen() {
        val w = CaptureWatchdog(errorLimit = 5, maxConsecutiveReopens = 2)
        val actions = (1..20).map { w.onReadError() }
        assertEquals(listOf(Action.REOPEN, Action.REOPEN, Action.GIVE_UP), actions.filter { it != Action.OK }.take(3))
        assertEquals(Action.REOPEN, actions[4])
    }

    @Test fun successfulReadResetsErrorCount() {
        val w = CaptureWatchdog(errorLimit = 5)
        repeat(4) { assertEquals(Action.OK, w.onReadError()) }
        w.onFrame(signal, 480, true)
        repeat(4) { assertEquals(Action.OK, w.onReadError()) }
    }
}
