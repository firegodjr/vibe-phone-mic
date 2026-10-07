package com.vibephonemic

import org.junit.Assert.assertEquals
import org.junit.Assert.assertNull
import org.junit.Test

class InputDevicesTest {
    private val builtIn = InputOption(id = 10, type = 15, address = "bottom", label = "Built-in microphone – bottom")
    private val usb = InputOption(id = 22, type = 11, address = "card=1;device=0", label = "USB microphone")

    @Test fun defaultSelectionResolvesToNull() {
        assertNull(InputDevices.resolve(UserSettings(), listOf(builtIn, usb)))
    }

    @Test fun specificSelectionById() {
        assertEquals(usb, InputDevices.resolve(UserSettings(inputDeviceId = 22, inputDeviceType = 11, inputDeviceAddress = "card=1;device=0"), listOf(builtIn, usb)))
    }

    @Test fun reattachedDeviceWithNewIdMatchesByTypeAndAddress() {
        val replugged = usb.copy(id = 37)
        assertEquals(replugged, InputDevices.resolve(UserSettings(inputDeviceId = 22, inputDeviceType = 11, inputDeviceAddress = "card=1;device=0"), listOf(builtIn, replugged)))
    }

    @Test fun disappearedDeviceFallsBackToDefault() {
        assertNull(InputDevices.resolve(UserSettings(inputDeviceId = 22, inputDeviceType = 11, inputDeviceAddress = "card=1;device=0"), listOf(builtIn)))
        assertNull(InputDevices.resolve(UserSettings(inputDeviceId = 22, inputDeviceType = 11), emptyList()))
    }

    @Test fun networkInterfaceLabels() {
        assertEquals("Wi-Fi", NetworkAddresses.labelFor("wlan0"))
        assertEquals("USB tethering", NetworkAddresses.labelFor("rndis0"))
        assertEquals("USB tethering", NetworkAddresses.labelFor("ncm0"))
        assertEquals("Hotspot", NetworkAddresses.labelFor("swlan0"))
        assertEquals("Ethernet", NetworkAddresses.labelFor("eth0"))
        assertEquals("Mobile data", NetworkAddresses.labelFor("rmnet_data0"))
        assertEquals("foo1", NetworkAddresses.labelFor("foo1"))
    }
}
