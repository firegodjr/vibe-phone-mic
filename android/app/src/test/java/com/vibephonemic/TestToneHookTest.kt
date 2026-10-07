package com.vibephonemic

import android.content.Intent
import com.vibephonemic.core.SineAudioSource
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class TestToneHookTest {
    @Before fun reset() = AppState.resetForTest()
    @After fun after() = AppState.resetForTest()

    private fun intent(tone: Boolean? = null, hz: Int? = null) = Intent().apply {
        tone?.let { putExtra(TestToneHook.EXTRA_TEST_TONE, it) }
        hz?.let { putExtra(TestToneHook.EXTRA_TEST_TONE_HZ, it) }
    }

    @Test fun debugBuildEnablesToneWithDefaultFrequency() {
        assertTrue(BuildConfig.DEBUG) // unit tests run against the debug variant
        assertTrue(TestToneHook.apply(intent(tone = true), isDebug = true))
        assertEquals(440, AppState.testToneHz)
    }

    @Test fun customFrequencyAndClamping() {
        TestToneHook.apply(intent(true, 1000), isDebug = true)
        assertEquals(1000, AppState.testToneHz)
        TestToneHook.apply(intent(true, 5), isDebug = true)
        assertEquals(20, AppState.testToneHz)
    }

    @Test fun falseRestoresMicrophone() {
        TestToneHook.apply(intent(true, 440), isDebug = true)
        assertTrue(TestToneHook.apply(intent(false), isDebug = true))
        assertNull(AppState.testToneHz)
    }

    @Test fun intentWithoutExtraIsIgnored() {
        AppState.testToneHz = 440
        assertFalse(TestToneHook.apply(intent(), isDebug = true))
        assertFalse(TestToneHook.apply(null, isDebug = true))
        assertEquals(440, AppState.testToneHz)
    }

    @Test fun releaseBuildIgnoresHook() {
        assertFalse(TestToneHook.apply(intent(true, 440), isDebug = false))
        assertNull(AppState.testToneHz)
        AppState.testToneHz = 440
        assertNull(TestToneHook.activeHz(isDebug = false))
        assertNull(TestToneHook.sourceFor(440, isDebug = false))
    }

    @Test fun sourceIsSineAtRequestedFrequency() {
        val src = TestToneHook.sourceFor(523, isDebug = true) as SineAudioSource
        assertEquals(523.0, src.frequencyHz, 0.0)
        assertEquals(48000, src.sampleRate)
        assertNull(TestToneHook.sourceFor(null, isDebug = true))
    }
}
