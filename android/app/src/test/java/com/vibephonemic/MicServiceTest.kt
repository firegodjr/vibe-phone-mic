package com.vibephonemic

import android.app.Application
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Looper
import androidx.test.core.app.ApplicationProvider
import com.vibephonemic.core.AudioSource
import com.vibephonemic.core.ClientMode
import com.vibephonemic.core.Json
import com.vibephonemic.core.Protocol
import com.vibephonemic.core.SineAudioSource
import com.vibephonemic.core.TestClient
import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import org.junit.runner.RunWith
import org.robolectric.Robolectric
import org.robolectric.RobolectricTestRunner
import org.robolectric.Shadows.shadowOf
import org.robolectric.android.controller.ServiceController
import org.robolectric.shadows.AudioDeviceInfoBuilder
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.util.concurrent.CopyOnWriteArrayList

/** Robolectric tests of the real foreground service with a fake microphone on ephemeral ports. */
@RunWith(RobolectricTestRunner::class)
class MicServiceTest {
    private val app: Application get() = ApplicationProvider.getApplicationContext()

    /** Every source the service asked for: (preferred device, label). */
    private val requested = CopyOnWriteArrayList<Pair<AudioDeviceInfo?, String>>()
    private val created = CopyOnWriteArrayList<CountingSource>()
    private var controller: ServiceController<MicService>? = null

    class CountingSource(label: String) : AudioSource {
        private val sine = SineAudioSource(440.0, 48000)
        override val sampleRate = 48000
        override val description = label
        @Volatile var starts = 0
        override fun start() { starts++; sine.start() }
        override fun read(buf: ShortArray, count: Int) = sine.read(buf, count)
        override fun stop() {}
    }

    @Before fun setUp() {
        AppState.resetForTest()
        app.getSharedPreferences("vpm_settings", Context.MODE_PRIVATE).edit().clear().commit()
        MicService.portOverride = 0
        MicService.sourceFactoryOverride = { device, label ->
            requested += device to label
            CountingSource(label).also { created += it }
        }
    }

    @After fun tearDown() {
        try { controller?.destroy() } catch (_: Exception) {}
        MicService.portOverride = null
        MicService.sourceFactoryOverride = null
        AppState.resetForTest()
    }

    private fun waitFor(what: String, timeoutMs: Long = 5000, cond: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (true) {
            shadowOf(Looper.getMainLooper()).idle()
            if (cond()) return
            if (System.currentTimeMillis() > deadline) fail("timed out waiting for $what")
            Thread.sleep(10)
        }
    }

    private fun startService(): MicService {
        val c = Robolectric.buildService(MicService::class.java, Intent(app, MicService::class.java))
        controller = c
        val service = c.create().startCommand(0, 1).get()
        waitFor("server running") { AppState.server.value.running }
        return service
    }

    private fun MicService.port(): Int = serverForTest!!.localPort

    @Test fun startGoesForegroundWithMicrophoneTypeAndListens() {
        val service = startService()
        val shadow = shadowOf(service)
        assertNotNull(shadow.lastForegroundNotification)
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE, service.foregroundServiceType)
        assertTrue(service.serverForTest!!.isRunning)
        assertEquals(48000, AppState.server.value.sampleRate)
        assertEquals(InputDevices.DEFAULT_LABEL, AppState.server.value.inputDescription)
        TestClient(service.port(), ClientMode.PROBE).use { c ->
            val hello = c.json(c.next())
            assertEquals(48000L, hello["sampleRate"])
        }
        // microphone path (no test tone): default device requested
        assertEquals(listOf<Pair<AudioDeviceInfo?, String>>(null to InputDevices.DEFAULT_LABEL), requested.toList())
        assertEquals(1, created.single().starts)
    }

    @Test fun discoveryRespondsWhileServiceRuns() {
        val service = startService()
        val port = service.discoveryForTest!!.localPort
        DatagramSocket().use { s ->
            s.soTimeout = 2000
            s.send(DatagramPacket(Protocol.DISCOVER_REQUEST, Protocol.DISCOVER_REQUEST.size, InetSocketAddress(InetAddress.getLoopbackAddress(), port)))
            val buf = ByteArray(512)
            val p = DatagramPacket(buf, buf.size)
            s.receive(p)
            val json = Json.parseFlat(String(buf, 0, p.length))
            assertEquals("vibe-phone-mic", json["service"])
            assertEquals(48777L, json["port"])
        }
    }

    @Test fun stopWhenNotRunningStillCallsStartForeground() {
        val c = Robolectric.buildService(MicService::class.java, Intent(app, MicService::class.java).setAction(MicService.ACTION_STOP))
        controller = c
        val service = c.create().startCommand(0, 1).get()
        val shadow = shadowOf(service)
        // startForeground must have been called (else ForegroundServiceDidNotStartInTimeException)
        assertEquals(1, shadow.lastForegroundNotificationId)
        assertEquals(ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE, service.foregroundServiceType)
        assertTrue(shadow.isForegroundStopped)
        assertTrue(shadow.isStoppedBySelf)
        assertNull(service.serverForTest)
        assertTrue(requested.isEmpty())
        assertTrue(AppState.userStopped)
    }

    @Test fun stopWhileRunningSendsByeStoppedAndStops() {
        val service = startService()
        val client = TestClient(service.port(), ClientMode.STREAM)
        client.next(); client.next() // HELLO, STATUS
        waitFor("client registered") { AppState.server.value.clientAddress == "127.0.0.1" }
        controller!!.withIntent(Intent(app, MicService::class.java).setAction(MicService.ACTION_STOP)).startCommand(0, 2)
        val shadow = shadowOf(service)
        assertTrue(shadow.isStoppedBySelf)
        assertTrue(shadow.isForegroundStopped)
        assertTrue(AppState.userStopped)
        controller!!.destroy()
        controller = null
        assertEquals(Protocol.BYE_STOPPED, client.expectBye())
        client.close()
        assertFalse(AppState.server.value.running)
    }

    @Test fun externalStartResetsUserStopped() {
        AppState.userStopped = true
        startService()
        assertFalse(AppState.userStopped)
    }

    @Test fun inputChangeRestartsCaptureWithPreferredDevice() {
        val usb = AudioDeviceInfoBuilder.newBuilder().setType(AudioDeviceInfo.TYPE_USB_DEVICE).build()
        shadowOf(app.getSystemService(AudioManager::class.java)).setInputDevices(listOf(usb))
        val service = startService()
        TestClient(service.port(), ClientMode.STREAM).use { c ->
            assertEquals(Protocol.TYPE_HELLO, c.next().type)
            assertEquals(InputDevices.DEFAULT_LABEL, c.json(c.next())["input"])
            AppState.update(app) {
                it.copy(inputDeviceId = 999, inputDeviceType = AudioDeviceInfo.TYPE_USB_DEVICE, inputDeviceAddress = usb.address ?: "")
            }
            waitFor("new source requested") { requested.size == 2 }
            val (device, label) = requested[1]
            assertEquals(AudioDeviceInfo.TYPE_USB_DEVICE, device!!.type)
            assertEquals(InputDevices.label(usb), label)
            // the capture thread opened the new source and STATUS announces it
            val status = c.json(c.nextOfType(Protocol.TYPE_STATUS))
            assertEquals(label, status["input"])
            assertEquals(1, created[1].starts)
        }
        // back to default
        AppState.update(app) { it.copy(inputDeviceId = 0, inputDeviceType = 0, inputDeviceAddress = "") }
        waitFor("default source requested") { requested.size == 3 }
        assertNull(requested[2].first)
    }

    @Test fun muteAndVolumeFromAppStateReachClients() {
        val service = startService()
        TestClient(service.port(), ClientMode.STREAM).use { c ->
            c.next()
            val st = c.json(c.next())
            assertEquals(false, st["muted"]); assertEquals(100L, st["volume"])
            AppState.update(app) { it.copy(muted = true) }
            shadowOf(Looper.getMainLooper()).idle()
            assertEquals(true, c.json(c.nextOfType(Protocol.TYPE_STATUS))["muted"])
            AppState.update(app) { it.copy(volume = 150) }
            shadowOf(Looper.getMainLooper()).idle()
            val st2 = c.json(c.nextOfType(Protocol.TYPE_STATUS))
            assertEquals(true, st2["muted"]); assertEquals(150L, st2["volume"])
        }
    }

    @Test fun persistedSettingsAppliedAtStart() {
        AppState.update(app) { it.copy(muted = true, volume = 40) }
        AppState.resetForTest()
        val service = startService()
        TestClient(service.port(), ClientMode.STREAM).use { c ->
            c.next()
            val st = c.json(c.next())
            assertEquals(true, st["muted"]); assertEquals(40L, st["volume"])
        }
    }

    @Test fun debugTestToneReplacesMicrophone() {
        AppState.testToneHz = 440
        val service = startService()
        assertTrue(requested.isEmpty())
        TestClient(service.port(), ClientMode.STREAM).use { c ->
            c.next()
            assertEquals("Test tone 440 Hz", c.json(c.next())["input"])
        }
        // switching the hook off at runtime goes back to the microphone
        AppState.testToneHz = null
        controller!!.withIntent(Intent(app, MicService::class.java)).startCommand(0, 2)
        waitFor("mic requested") { requested.size == 1 }
        assertNull(requested[0].first)
    }
}
