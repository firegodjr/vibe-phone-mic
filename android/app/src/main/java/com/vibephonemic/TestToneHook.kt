package com.vibephonemic

import android.content.Intent
import com.vibephonemic.core.AudioSource
import com.vibephonemic.core.SineAudioSource

/**
 * Debug-only e2e hook: `adb shell am start -n com.vibephonemic/.MainActivity --ez test_tone true --ei test_tone_hz 440`
 * replaces the microphone with a generated sine. `--ez test_tone false` restores the microphone.
 * Inert in release builds (guarded by BuildConfig.DEBUG).
 */
object TestToneHook {
    const val EXTRA_TEST_TONE = "test_tone"
    const val EXTRA_TEST_TONE_HZ = "test_tone_hz"
    const val DEFAULT_HZ = 440

    /** Applies the intent extras to [AppState.testToneHz]. Returns true if the intent carried the hook. */
    fun apply(intent: Intent?, isDebug: Boolean = BuildConfig.DEBUG): Boolean {
        if (!isDebug || intent == null || !intent.hasExtra(EXTRA_TEST_TONE)) return false
        AppState.testToneHz = if (intent.getBooleanExtra(EXTRA_TEST_TONE, false)) {
            intent.getIntExtra(EXTRA_TEST_TONE_HZ, DEFAULT_HZ).coerceIn(20, 20000)
        } else {
            null
        }
        return true
    }

    fun activeHz(isDebug: Boolean = BuildConfig.DEBUG): Int? = if (isDebug) AppState.testToneHz else null

    /** The source replacing the microphone, or null to use the real microphone. */
    fun sourceFor(hz: Int?, isDebug: Boolean = BuildConfig.DEBUG): AudioSource? =
        if (isDebug && hz != null) SineAudioSource(hz.toDouble()) else null
}
