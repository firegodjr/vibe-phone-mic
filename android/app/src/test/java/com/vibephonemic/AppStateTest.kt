package com.vibephonemic

import android.content.Context
import androidx.test.core.app.ApplicationProvider
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class AppStateTest {
    private val ctx: Context get() = ApplicationProvider.getApplicationContext()

    @Before fun reset() = AppState.resetForTest()
    @After fun after() = AppState.resetForTest()

    @Test fun defaultsWhenNothingSaved() {
        AppState.load(ctx)
        assertEquals(UserSettings(), AppState.settings.value)
    }

    @Test fun settingsSurviveProcessRestart() {
        AppState.load(ctx)
        AppState.update(ctx) { it.copy(muted = true, volume = 175, inputDeviceId = 22, inputDeviceType = 11, inputDeviceAddress = "card=1") }
        AppState.resetForTest() // simulate a fresh process
        assertEquals(UserSettings(), AppState.settings.value)
        AppState.load(ctx)
        assertEquals(UserSettings(true, 175, 22, 11, "card=1"), AppState.settings.value)
    }

    @Test fun selectingDefaultIsPersisted() {
        AppState.update(ctx) { it.copy(inputDeviceId = 22, inputDeviceType = 11) }
        AppState.update(ctx) { it.copy(inputDeviceId = 0, inputDeviceType = 0, inputDeviceAddress = "") }
        AppState.resetForTest()
        AppState.load(ctx)
        assertEquals(0, AppState.settings.value.inputDeviceId)
    }

    @Test fun outOfRangeVolumeOnDiskIsClamped() {
        ctx.getSharedPreferences("vpm_settings", Context.MODE_PRIVATE).edit().putInt("volume", 999).commit()
        AppState.load(ctx)
        assertEquals(200, AppState.settings.value.volume)
    }
}
