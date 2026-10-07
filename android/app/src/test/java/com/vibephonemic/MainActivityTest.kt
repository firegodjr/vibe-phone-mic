package com.vibephonemic

import android.Manifest
import android.app.Application
import android.content.Context
import android.content.Intent
import androidx.test.core.app.ActivityScenario
import androidx.test.core.app.ApplicationProvider
import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.assertIsOn
import androidx.compose.ui.test.junit4.createEmptyComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.performClick
import androidx.compose.ui.test.performScrollTo
import androidx.compose.ui.test.assertTextEquals
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Before
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf

/** Drives the real activity (Robolectric): permission flow, auto-start, test-tone hook, navigation, persistence. */
@RunWith(RobolectricTestRunner::class)
class MainActivityTest {
    @get:Rule val rule = createEmptyComposeRule()

    private val app: Application get() = ApplicationProvider.getApplicationContext()

    @Before fun setUp() {
        AppState.resetForTest()
        app.getSharedPreferences("vpm_settings", Context.MODE_PRIVATE).edit().clear().commit()
    }

    @After fun tearDown() = AppState.resetForTest()

    private fun grantMic() = shadowOf(app).grantPermissions(Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS)

    private fun startedServiceIntent(): Intent? {
        var i: Intent? = shadowOf(app).nextStartedService
        while (i != null && i.component?.className != MicService::class.java.name) i = shadowOf(app).nextStartedService
        return i
    }

    @Test fun permissionDeniedShowsCardAndDoesNotStartServer() {
        ActivityScenario.launch(MainActivity::class.java).use {
            rule.onNodeWithTag("permission_card").assertIsDisplayed()
            rule.onNodeWithTag("connection_status").assertTextEquals("Server stopped")
            assertNull(startedServiceIntent())
        }
    }

    @Test fun permissionGrantedAutoStartsServer() {
        grantMic()
        ActivityScenario.launch(MainActivity::class.java).use {
            rule.onAllNodesWithTag("permission_card").assertCountEquals(0)
            rule.onNodeWithTag("port_text").assertTextEquals("48777")
            assertNotNull("MicService should be started", startedServiceIntent())
        }
    }

    @Test fun userStoppedServerIsNotRestartedOnReturn() {
        grantMic()
        AppState.userStopped = true
        ActivityScenario.launch(MainActivity::class.java).use {
            assertNull(startedServiceIntent())
        }
    }

    @Test fun testToneIntentIsApplied() {
        grantMic()
        val intent = Intent(app, MainActivity::class.java)
            .putExtra(TestToneHook.EXTRA_TEST_TONE, true)
            .putExtra(TestToneHook.EXTRA_TEST_TONE_HZ, 1000)
        ActivityScenario.launch<MainActivity>(intent).use {
            assertEquals(1000, AppState.testToneHz)
            assertNotNull(startedServiceIntent())
        }
    }

    @Test fun launchWithoutHookUsesMicrophone() {
        grantMic()
        ActivityScenario.launch(MainActivity::class.java).use {
            val started = startedServiceIntent()
            assertNotNull(started)
            assertTrue(started!!.extras == null || !started.hasExtra(TestToneHook.EXTRA_TEST_TONE))
            assertNull(AppState.testToneHz)
            // what MicService will use: no generated source -> the real AudioRecord path
            assertNull(TestToneHook.sourceFor(TestToneHook.activeHz()))
        }
    }

    @Test fun relaunchWithoutHookKeepsActiveToneButFalseClearsIt() {
        grantMic()
        val tone = Intent(app, MainActivity::class.java).putExtra(TestToneHook.EXTRA_TEST_TONE, true)
        ActivityScenario.launch<MainActivity>(tone).use { scenario ->
            assertEquals(TestToneHook.DEFAULT_HZ, AppState.testToneHz)
            scenario.onActivity { it.onNewIntent(Intent(app, MainActivity::class.java)) }
            assertEquals(TestToneHook.DEFAULT_HZ, AppState.testToneHz)
            scenario.onActivity { it.onNewIntent(Intent(app, MainActivity::class.java).putExtra(TestToneHook.EXTRA_TEST_TONE, false)) }
            assertNull(AppState.testToneHz)
        }
    }

    @Test fun settingsNavigationAndBack() {
        ActivityScenario.launch(MainActivity::class.java).use {
            rule.onNodeWithTag("settings_button").performClick()
            rule.onNodeWithText("Settings").assertIsDisplayed()
            rule.onNodeWithTag("input_option_default").assertIsDisplayed()
            rule.onNodeWithTag("back_button").performClick()
            rule.onNodeWithTag("port_text").performScrollTo().assertIsDisplayed()
        }
    }

    @Test fun muteAndVolumePersistAcrossRecreate() {
        ActivityScenario.launch(MainActivity::class.java).use { scenario ->
            rule.onNodeWithTag("mute_button").performScrollTo().performClick()
            rule.onNodeWithTag("volume_up").performScrollTo().performClick()
            assertTrue(AppState.settings.value.muted)
            assertEquals(110, AppState.settings.value.volume)
            AppState.resetForTest() // simulate process death: state must come back from disk
            scenario.recreate()
            rule.onNodeWithTag("mute_button").performScrollTo().assertIsOn()
            rule.onNodeWithTag("volume_text").performScrollTo().assertTextEquals("110 %")
        }
    }
}
