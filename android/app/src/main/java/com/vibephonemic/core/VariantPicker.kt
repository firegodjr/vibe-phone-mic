package com.vibephonemic.core

import kotlin.random.Random

/**
 * Picks which recorded sample of a multi-sampled sound to play next: random, but never the same one twice in a
 * row, so repeated presses never sound identical (button-sounds style). Keep one picker per sound and direction
 * (press/release).
 */
class VariantPicker(val count: Int, private val random: Random = Random.Default) {
    init {
        require(count > 0) { "count must be > 0" }
    }

    var last: Int = -1
        private set

    fun next(): Int {
        val i = if (count == 1 || last < 0) {
            random.nextInt(count)
        } else {
            // Draw from the other count-1 samples, skipping the last one.
            random.nextInt(count - 1).let { if (it >= last) it + 1 else it }
        }
        last = i
        return i
    }
}
