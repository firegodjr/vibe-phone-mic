package com.vibephonemic.ui

import androidx.compose.ui.test.assertCountEquals
import androidx.compose.ui.test.assertIsNotSelected
import androidx.compose.ui.test.assertIsSelected
import androidx.compose.ui.test.junit4.createComposeRule
import androidx.compose.ui.test.onAllNodesWithTag
import androidx.compose.ui.test.onNodeWithTag
import androidx.compose.ui.test.onNodeWithText
import androidx.compose.ui.test.assertIsDisplayed
import androidx.compose.ui.test.performClick
import com.vibephonemic.InputOption
import org.junit.Assert.assertEquals
import org.junit.Rule
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.RobolectricTestRunner

@RunWith(RobolectricTestRunner::class)
class SettingsScreenTest {
    @get:Rule val rule = createComposeRule()

    private val builtIn = InputOption(10, 15, "bottom", "Built-in microphone – bottom")
    private val usb = InputOption(22, 11, "card=1", "USB microphone")

    private val selections = mutableListOf<InputOption?>()
    private var backs = 0

    private fun show(options: List<InputOption>, selectedId: Int) = rule.setContent {
        VpmTheme {
            SettingsScreen(options, selectedId, "1.0.0", onSelect = { selections += it }, onBack = { backs++ })
        }
    }

    @Test fun defaultInputSelected() {
        show(listOf(builtIn, usb), 0)
        rule.onNodeWithTag("input_option_default").assertIsSelected()
        rule.onNodeWithTag("input_option_10").assertIsNotSelected()
        rule.onNodeWithTag("input_option_22").assertIsNotSelected()
        rule.onNodeWithText("Default microphone").assertIsDisplayed()
        rule.onNodeWithText("USB microphone").assertIsDisplayed()
    }

    @Test fun specificInputSelected() {
        show(listOf(builtIn, usb), 22)
        rule.onNodeWithTag("input_option_22").assertIsSelected()
        rule.onNodeWithTag("input_option_default").assertIsNotSelected()
    }

    @Test fun disappearedDeviceFallsBackToDefault() {
        show(listOf(builtIn), 22)
        rule.onAllNodesWithTag("input_option_22").assertCountEquals(0)
        rule.onNodeWithTag("input_option_default").assertIsSelected()
    }

    @Test fun noDevicesStillOffersDefault() {
        show(emptyList(), 0)
        rule.onNodeWithTag("input_option_default").assertIsSelected()
    }

    @Test fun selectingReportsChoice() {
        show(listOf(builtIn, usb), 0)
        rule.onNodeWithTag("input_option_22").performClick()
        rule.onNodeWithTag("input_option_default").performClick()
        assertEquals(listOf(usb, null), selections)
    }

    @Test fun backButton() {
        show(listOf(builtIn), 0)
        rule.onNodeWithTag("back_button").performClick()
        assertEquals(1, backs)
    }
}
