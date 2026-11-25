package com.vibephonemic.core

import org.junit.Assert.assertFalse
import org.junit.Assert.assertTrue
import org.junit.Test

class SilenceGateTest {
    private var now = 1_000_000_000L
    private val gate = SilenceGate { now }
    private fun advanceMs(ms: Long) { now += ms * 1_000_000 }

    @Test fun openUntilHeldThenClosesForTheDuration() {
        assertFalse(gate.isClosed)
        gate.hold(500)
        assertTrue(gate.isClosed)
        advanceMs(499)
        assertTrue(gate.isClosed)
        advanceMs(1)
        assertFalse(gate.isClosed)
    }

    @Test fun overlappingHoldsExtendButNeverShorten() {
        gate.hold(500)
        advanceMs(300)
        gate.hold(500) // press then release: now open at 800 ms
        advanceMs(400)
        assertTrue(gate.isClosed)
        gate.hold(50) // shorter hold must not cut the pending one
        advanceMs(60)
        assertTrue(gate.isClosed)
        advanceMs(40)
        assertFalse(gate.isClosed)
    }

    @Test fun worksAcrossNanoTimeWraparound() {
        now = Long.MAX_VALUE - 100_000_000 // 100 ms before overflow
        gate.hold(500)
        advanceMs(300)
        assertTrue(gate.isClosed)
        advanceMs(250)
        assertFalse(gate.isClosed)
    }
}
