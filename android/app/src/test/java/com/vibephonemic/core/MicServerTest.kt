package com.vibephonemic.core

import org.junit.After
import org.junit.Assert.assertEquals
import org.junit.Assert.assertNotNull
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Assert.fail
import org.junit.Before
import org.junit.Test
import java.io.DataInputStream
import java.io.EOFException
import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetAddress
import java.net.InetSocketAddress
import java.net.Socket
import java.net.SocketTimeoutException
import java.util.concurrent.CopyOnWriteArrayList
import java.util.concurrent.LinkedBlockingQueue
import java.util.concurrent.TimeUnit

/** Minimal Kotlin protocol client used to drive the real server over sockets. */
class TestClient(port: Int, mode: ClientMode) : AutoCloseable {
    val socket = Socket().apply {
        connect(InetSocketAddress(InetAddress.getLoopbackAddress(), port), 2000)
        soTimeout = 3000
        getOutputStream().write(Handshake.encode(mode))
        getOutputStream().flush()
    }
    private val input = DataInputStream(socket.getInputStream())

    fun next(): Frame = FrameCodec.read(input)

    fun json(f: Frame): Map<String, Any?> = Json.parseFlat(String(f.payload, Charsets.UTF_8))

    fun nextOfType(type: Byte, max: Int = 500): Frame {
        repeat(max) { val f = next(); if (f.type == type) return f }
        fail("no frame of type $type"); throw IllegalStateException()
    }

    /** True if the server closed the connection (EOF) within the timeout, draining any pending frames. */
    fun closedByServer(): Boolean = try {
        repeat(10_000) { next() }
        false
    } catch (_: EOFException) {
        true
    } catch (_: SocketTimeoutException) {
        false
    } catch (_: IOException) {
        true // connection reset
    }

    /** Reads (skipping audio/status) until BYE; returns its reason, then asserts the server closes. */
    fun expectBye(): String {
        repeat(10_000) {
            val f = next()
            if (f.type == Protocol.TYPE_BYE) {
                val reason = json(f)["reason"] as String
                if (!closedByServer()) fail("socket not closed after BYE")
                return reason
            }
        }
        fail("no BYE"); throw IllegalStateException()
    }

    override fun close() = socket.close()
}

/** Scriptable fake microphone for watchdog tests. */
class FakeMic(
    override val watchdogEnabled: Boolean = true,
    private val behaviour: (start: Int, read: Long) -> Int, // returns: 1 = signal, 0 = zeros, -1 = error
) : AudioSource {
    override val sampleRate = 48000
    override val description = "Fake mic"
    val starts = java.util.concurrent.atomic.AtomicInteger(0)
    val reads = java.util.concurrent.atomic.AtomicLong(0)
    private var readsSinceStart = 0L

    override fun start() { starts.incrementAndGet(); readsSinceStart = 0 }

    override fun read(buf: ShortArray, count: Int): Int {
        Thread.sleep(0, 200_000)
        reads.incrementAndGet()
        return when (behaviour(starts.get(), readsSinceStart++)) {
            1 -> { for (i in 0 until count) buf[i] = (if (i % 2 == 0) 8000 else -8000).toShort(); count }
            0 -> { java.util.Arrays.fill(buf, 0, count, 0); count }
            else -> -3
        }
    }

    override fun stop() {}
}

class MicServerTest {
    private val clientEvents = LinkedBlockingQueue<String>()
    private val sourceErrors = CopyOnWriteArrayList<String?>()
    private lateinit var server: MicServer

    private val listener = object : MicServer.Listener {
        override fun onClientChanged(address: String?) { clientEvents.put(address ?: "<none>") }
        override fun onSourceChanged(sampleRate: Int, description: String, error: String?) { sourceErrors.add(error) }
    }

    private val warnings = CopyOnWriteArrayList<String>()

    private fun startServer(
        source: AudioSource = SineAudioSource(440.0, 48000),
        watchdog: () -> CaptureWatchdog = { CaptureWatchdog() },
    ) {
        val l = object : MicServer.Listener by listener {
            override fun onWarning(message: String) { warnings.add(message) }
        }
        server = MicServer(port = 0, deviceName = "Test Phone", initialSource = source, listener = l, watchdogFactory = watchdog)
        server.start()
        assertTrue(server.localPort > 0)
    }

    @Before fun setUp() { clientEvents.clear() }

    @After fun tearDown() { if (::server.isInitialized) server.stop() }

    private fun TestClient.expectHelloAndStatus(rate: Int = 48000): Map<String, Any?> {
        val hello = next()
        assertEquals(Protocol.TYPE_HELLO, hello.type)
        val h = json(hello)
        assertEquals("Test Phone", h["name"])
        assertEquals(rate.toLong(), h["sampleRate"])
        assertEquals(1L, h["channels"])
        assertEquals("s16le", h["format"])
        assertEquals(10L, h["frameMs"])
        assertEquals(1L, h["version"])
        val status = next()
        assertEquals(Protocol.TYPE_STATUS, status.type)
        return json(status)
    }

    @Test fun streamClientGetsHelloStatusAndSequentialAudio() {
        startServer()
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            val st = c.expectHelloAndStatus()
            assertEquals(false, st["muted"])
            assertEquals(100L, st["volume"])
            assertEquals("Test tone 440 Hz", st["input"])
            assertEquals("127.0.0.1", clientEvents.poll(2, TimeUnit.SECONDS))

            var lastSeq = -1L
            var lastTs = 0L
            repeat(30) {
                val f = c.next()
                assertEquals(Protocol.TYPE_AUDIO, f.type)
                val a = FrameCodec.decodeAudio(f.payload)
                assertEquals(480, a.samples.size)
                assertEquals(lastSeq + 1, a.seq)
                assertTrue(a.captureTimeMicros >= lastTs)
                lastSeq = a.seq
                lastTs = a.captureTimeMicros
                assertTrue("audio should be non-silent", AudioMath.rms(a.samples, a.samples.size) > 1000)
            }
        }
        assertEquals("<none>", clientEvents.poll(2, TimeUnit.SECONDS))
        assertNull(server.clientAddress)
    }

    @Test fun probeClientGetsHelloThenClose() {
        startServer()
        TestClient(server.localPort, ClientMode.PROBE).use { c ->
            val hello = c.next()
            assertEquals(Protocol.TYPE_HELLO, hello.type)
            assertEquals(48000L, c.json(hello)["sampleRate"])
            try { c.next(); fail("expected EOF after probe HELLO") } catch (_: EOFException) {}
        }
        assertNull(clientEvents.poll(300, TimeUnit.MILLISECONDS))
    }

    @Test fun badHandshakeIsClosed() {
        startServer()
        Socket("127.0.0.1", server.localPort).use { s ->
            s.soTimeout = 3000
            s.getOutputStream().write("GET / HTTP".toByteArray())
            assertEquals(-1, s.getInputStream().read())
        }
    }

    @Test fun muteSendsStatusAndSilenceButFramesKeepFlowing() {
        startServer()
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            c.expectHelloAndStatus()
            c.nextOfType(Protocol.TYPE_AUDIO)
            server.setMuted(true)
            val st = c.json(c.nextOfType(Protocol.TYPE_STATUS))
            assertEquals(true, st["muted"])
            // allow a frame or two captured before the change, then expect silence with continuing seq
            repeat(3) { c.nextOfType(Protocol.TYPE_AUDIO) }
            var prev = -1L
            repeat(20) {
                val a = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload)
                assertEquals(0.0, AudioMath.rms(a.samples, a.samples.size), 0.0)
                if (prev >= 0) assertEquals(prev + 1, a.seq)
                prev = a.seq
            }
            server.setMuted(true) // no change -> no STATUS
            server.setVolume(50)
            val st2 = c.json(c.nextOfType(Protocol.TYPE_STATUS))
            assertEquals(true, st2["muted"])
            assertEquals(50L, st2["volume"])
            server.setMuted(false)
            assertEquals(false, c.json(c.nextOfType(Protocol.TYPE_STATUS))["muted"])
            repeat(3) { c.nextOfType(Protocol.TYPE_AUDIO) }
            val a = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload)
            val rms = AudioMath.rms(a.samples, a.samples.size)
            // sine amplitude 0.5 * 32767 at 50% gain -> rms ~ 5790
            assertTrue("rms $rms", rms in 5000.0..6600.0)
        }
    }

    @Test fun newStreamClientReplacesOld() {
        startServer()
        val a = TestClient(server.localPort, ClientMode.STREAM)
        a.expectHelloAndStatus()
        a.nextOfType(Protocol.TYPE_AUDIO)
        assertEquals("127.0.0.1", clientEvents.poll(2, TimeUnit.SECONDS))

        TestClient(server.localPort, ClientMode.STREAM).use { b ->
            b.expectHelloAndStatus()
            assertTrue("old client should be closed", a.closedByServer())
            val first = FrameCodec.decodeAudio(b.nextOfType(Protocol.TYPE_AUDIO).payload)
            assertEquals(0L, first.seq)
            val second = FrameCodec.decodeAudio(b.nextOfType(Protocol.TYPE_AUDIO).payload)
            assertEquals(1L, second.seq)
            assertEquals("127.0.0.1", server.clientAddress)
        }
        a.close()
    }

    @Test fun probeDoesNotDisturbStreamClient() {
        startServer()
        TestClient(server.localPort, ClientMode.STREAM).use { s ->
            s.expectHelloAndStatus()
            TestClient(server.localPort, ClientMode.PROBE).use { p ->
                assertEquals(Protocol.TYPE_HELLO, p.next().type)
            }
            var prev = FrameCodec.decodeAudio(s.nextOfType(Protocol.TYPE_AUDIO).payload).seq
            repeat(10) {
                val seq = FrameCodec.decodeAudio(s.nextOfType(Protocol.TYPE_AUDIO).payload).seq
                assertEquals(prev + 1, seq)
                prev = seq
            }
        }
    }

    @Test fun failingSourceFallsBackToSilence() {
        val broken = object : AudioSource {
            override val sampleRate = 48000
            override val description = "broken"
            override fun start() = throw IllegalStateException("mic busy")
            override fun read(buf: ShortArray, count: Int) = -1
            override fun stop() {}
        }
        startServer(broken)
        assertEquals("mic busy", sourceErrors.last())
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            assertEquals("Microphone unavailable", c.expectHelloAndStatus()["input"])
            val a = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload)
            assertEquals(480, a.samples.size)
        }
    }

    @Test fun switchingSourceToNewRateReconnectsWithNewHello() {
        startServer()
        val c = TestClient(server.localPort, ClientMode.STREAM)
        c.expectHelloAndStatus()
        server.setAudioSource(SineAudioSource(1000.0, 44100))
        assertTrue(c.closedByServer())
        c.close()
        Thread.sleep(50)
        TestClient(server.localPort, ClientMode.STREAM).use { d ->
            val st = d.expectHelloAndStatus(rate = 44100)
            assertEquals("Test tone 1000 Hz", st["input"])
            assertEquals(441, FrameCodec.decodeAudio(d.nextOfType(Protocol.TYPE_AUDIO).payload).samples.size)
        }
    }

    @Test fun listeningWithoutClientCapturesAndReportsLevel() {
        val levels = LinkedBlockingQueue<Float>()
        server = MicServer(0, "Test Phone", SineAudioSource(440.0, 48000), object : MicServer.Listener {
            override fun onLevel(level: Float) { levels.put(level) }
        })
        assertTrue(!server.isRunning)
        server.start()
        assertTrue(server.isRunning)
        assertNull(server.clientAddress)
        assertEquals(48000, server.currentSampleRate)
        val l = levels.poll(2, TimeUnit.SECONDS)
        assertNotNull(l)
        assertTrue("level $l", l!! > 0.5f)
        server.setMuted(true)
        Thread.sleep(150)
        levels.clear()
        assertEquals(0f, levels.poll(2, TimeUnit.SECONDS)!!, 0f)
    }

    @Test fun clientDisconnectReturnsToListening() {
        startServer()
        TestClient(server.localPort, ClientMode.STREAM).use { it.expectHelloAndStatus() }
        assertEquals("127.0.0.1", clientEvents.poll(2, TimeUnit.SECONDS))
        assertEquals("<none>", clientEvents.poll(2, TimeUnit.SECONDS))
        assertNull(server.clientAddress)
        assertTrue(server.isRunning)
        // a new client is accepted afterwards with a fresh sequence
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            c.expectHelloAndStatus()
            assertEquals(0L, FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload).seq)
        }
    }

    private fun steadyRms(c: TestClient): Double {
        repeat(4) { c.nextOfType(Protocol.TYPE_AUDIO) }
        val a = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload)
        return AudioMath.rms(a.samples, a.samples.size)
    }

    @Test fun volumeZeroHundredTwoHundredAndClipping() {
        startServer(SineAudioSource(440.0, 48000, amplitude = 0.8))
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            assertEquals(100L, c.expectHelloAndStatus()["volume"])
            val unity = steadyRms(c)
            assertEquals(0.8 * 32767 / Math.sqrt(2.0), unity, 600.0)

            server.setVolume(0)
            assertEquals(0L, c.json(c.nextOfType(Protocol.TYPE_STATUS))["volume"])
            assertEquals(0.0, steadyRms(c), 0.0)

            server.setVolume(200)
            assertEquals(200L, c.json(c.nextOfType(Protocol.TYPE_STATUS))["volume"])
            repeat(4) { c.nextOfType(Protocol.TYPE_AUDIO) }
            val loud = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload).samples
            // 0.8 * 2 = 1.6x full scale: must clip at the rails, never wrap around
            assertEquals(Short.MAX_VALUE, loud.max())
            assertEquals(Short.MIN_VALUE, loud.min())
            assertTrue(AudioMath.rms(loud, loud.size) > unity)

            server.setVolume(250) // clamped to 200 -> no change, no STATUS
            server.setVolume(100)
            assertEquals(100L, c.json(c.nextOfType(Protocol.TYPE_STATUS))["volume"])
        }
    }

    @Test fun replacedClientGetsByeReplaced() {
        startServer()
        val a = TestClient(server.localPort, ClientMode.STREAM)
        a.expectHelloAndStatus()
        a.nextOfType(Protocol.TYPE_AUDIO)
        TestClient(server.localPort, ClientMode.STREAM).use { b ->
            b.expectHelloAndStatus()
            assertEquals(Protocol.BYE_REPLACED, a.expectBye())
            // the new client never sees a BYE while streaming
            repeat(20) { assertTrue(b.next().type != Protocol.TYPE_BYE) }
        }
        a.close()
    }

    @Test fun stopSendsByeStopped() {
        startServer()
        val c = TestClient(server.localPort, ClientMode.STREAM)
        c.expectHelloAndStatus()
        c.nextOfType(Protocol.TYPE_AUDIO)
        server.stop()
        assertEquals(Protocol.BYE_STOPPED, c.expectBye())
        c.close()
    }

    private fun waitUntil(timeoutMs: Long = 5000, cond: () -> Boolean) {
        val deadline = System.currentTimeMillis() + timeoutMs
        while (!cond()) {
            if (System.currentTimeMillis() > deadline) fail("condition not met in time")
            Thread.sleep(5)
        }
    }

    @Test fun watchdogReopensMicThatDeliversDigitalSilence() {
        // first open is wedged (zeros forever), the reopened one works
        val mic = FakeMic { start, _ -> if (start == 1) 0 else 1 }
        startServer(mic) { CaptureWatchdog(zeroFramesLimit = 20) }
        waitUntil { mic.starts.get() == 2 }
        assertTrue(warnings.any { it.contains("digital silence") })
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            c.expectHelloAndStatus()
            val a = FrameCodec.decodeAudio(c.nextOfType(Protocol.TYPE_AUDIO).payload)
            assertTrue(AudioMath.rms(a.samples, a.samples.size) > 1000)
        }
        assertEquals(2, mic.starts.get())
    }

    @Test fun watchdogIgnoresSilenceWhileMuted() {
        val mic = FakeMic { _, _ -> 0 }
        server = MicServer(0, "Test Phone", mic, watchdogFactory = { CaptureWatchdog(zeroFramesLimit = 20) })
        server.setMuted(true)
        server.start()
        waitUntil { mic.reads.get() > 300 }
        assertEquals(1, mic.starts.get())
    }

    @Test fun watchdogIgnoresGeneratedSources() {
        val tone = FakeMic(watchdogEnabled = false) { _, _ -> 0 }
        startServer(tone) { CaptureWatchdog(zeroFramesLimit = 20) }
        waitUntil { tone.reads.get() > 300 }
        assertEquals(1, tone.starts.get())
    }

    @Test fun watchdogReopensAfterReadErrors() {
        val mic = FakeMic { start, _ -> if (start == 1) -1 else 1 }
        startServer(mic)
        waitUntil { mic.starts.get() == 2 }
        assertTrue(warnings.any { it.contains("read failed") })
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            assertEquals("Fake mic", c.expectHelloAndStatus()["input"])
        }
    }

    @Test fun watchdogRetriesAreBounded() {
        val mic = FakeMic { _, _ -> 0 }
        startServer(mic) { CaptureWatchdog(zeroFramesLimit = 10, maxConsecutiveReopens = 3) }
        waitUntil { warnings.any { it.contains("giving up") } }
        val readsAtGiveUp = mic.reads.get()
        waitUntil { mic.reads.get() > readsAtGiveUp + 200 }
        assertEquals(4, mic.starts.get()) // initial + 3 reopens, then no more
    }

    @Test fun persistentReadErrorsFallBackToSilence() {
        val mic = FakeMic { _, _ -> -1 }
        startServer(mic) { CaptureWatchdog(errorLimit = 3, maxConsecutiveReopens = 2) }
        waitUntil { sourceErrors.size >= 2 }
        TestClient(server.localPort, ClientMode.STREAM).use { c ->
            assertEquals("Microphone unavailable", c.expectHelloAndStatus()["input"])
        }
        assertEquals(3, mic.starts.get())
    }

    @Test fun stopClosesClients() {
        startServer()
        val c = TestClient(server.localPort, ClientMode.STREAM)
        c.expectHelloAndStatus()
        server.stop()
        assertTrue(c.closedByServer())
        c.close()
        assertTrue(!server.isRunning)
    }
}

class DiscoveryResponderTest {
    @Test fun udpRoundTripOnLoopback() {
        val responder = DiscoveryResponder(bindPort = 0, advertisedPort = 48777) { "Loopback Phone" }
        responder.start()
        try {
            DatagramSocket().use { sock ->
                sock.soTimeout = 2000
                val target = InetSocketAddress(InetAddress.getLoopbackAddress(), responder.localPort)

                // Garbage is ignored.
                sock.send(DatagramPacket("hello".toByteArray(), 5, target))
                val buf = ByteArray(1024)
                try {
                    sock.soTimeout = 300
                    sock.receive(DatagramPacket(buf, buf.size)); fail("unexpected reply to garbage")
                } catch (_: SocketTimeoutException) {}

                sock.soTimeout = 2000
                sock.send(DatagramPacket(Protocol.DISCOVER_REQUEST, Protocol.DISCOVER_REQUEST.size, target))
                val reply = DatagramPacket(buf, buf.size)
                sock.receive(reply)
                val json = Json.parseFlat(String(reply.data, 0, reply.length, Charsets.UTF_8))
                assertEquals("vibe-phone-mic", json["service"])
                assertEquals(1L, json["version"])
                assertEquals("Loopback Phone", json["name"])
                assertEquals(48777L, json["port"])
                assertNotNull(reply.socketAddress)
            }
        } finally {
            responder.stop()
        }
    }
}
