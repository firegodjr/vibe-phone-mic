package com.vibephonemic.core

import kotlin.random.Random
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotEquals
import org.junit.Assert.assertTrue
import org.junit.Test

class VariantPickerTest {
    @Test fun neverRepeatsAndCoversAllSamples() {
        val p = VariantPicker(5, Random(42))
        val seen = mutableSetOf<Int>()
        var prev = -1
        repeat(500) {
            val i = p.next()
            assertTrue(i in 0 until 5)
            assertNotEquals(prev, i)
            seen += i
            prev = i
        }
        assertEquals(setOf(0, 1, 2, 3, 4), seen)
    }

    @Test fun singleSampleAlwaysZero() {
        val p = VariantPicker(1)
        repeat(5) { assertEquals(0, p.next()) }
    }

    @Test(expected = IllegalArgumentException::class) fun emptyRejected() {
        VariantPicker(0)
    }
}
