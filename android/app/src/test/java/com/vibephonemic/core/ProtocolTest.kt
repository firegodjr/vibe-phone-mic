package com.vibephonemic.core

import org.junit.Assert.assertArrayEquals
import org.junit.Assert.assertEquals
import org.junit.Assert.assertFalse
import org.junit.Assert.assertNull
import org.junit.Assert.assertTrue
import org.junit.Test
import java.io.ByteArrayInputStream
import java.io.ByteArrayOutputStream
import java.io.EOFException
import java.nio.ByteBuffer

class ProtocolTest {
    @Test fun handshakeEncodeIsExactBytes() {
        assertArrayEquals(byteArrayOf('V'.code.toByte(), 'P'.code.toByte(), 'M'.code.toByte(), 'C'.code.toByte(), 1, 0, 0, 0), Handshake.encode(ClientMode.STREAM))
        assertArrayEquals(byteArrayOf(0x56, 0x50, 0x4D, 0x43, 1, 1, 0, 0), Handshake.encode(ClientMode.PROBE))
    }

    @Test fun handshakeParsesValidModes() {
        assertEquals(ClientMode.STREAM, Handshake.parse(Handshake.encode(ClientMode.STREAM)))
        assertEquals(ClientMode.PROBE, Handshake.parse(Handshake.encode(ClientMode.PROBE)))
    }

    @Test fun handshakeRejectsInvalid() {
        assertNull(Handshake.parse("XPMC".toByteArray() + byteArrayOf(1, 0, 0, 0)))
        assertNull(Handshake.parse(Handshake.encode(ClientMode.STREAM, version = 2)))
        assertNull(Handshake.parse("VPMC".toByteArray() + byteArrayOf(1, 2, 0, 0)))
        assertNull(Handshake.parse("VPMC".toByteArray() + byteArrayOf(1, 0, 0)))
        assertNull(Handshake.parse("GET / HTTP/1.1".toByteArray()))
    }

    @Test fun headerIsBigEndianTypeAndLength() {
        val f = FrameCodec.encode(Protocol.TYPE_HELLO, byteArrayOf(1, 2, 3))
        assertArrayEquals(byteArrayOf(0x01, 0, 0, 0, 3, 1, 2, 3), f)
    }

    @Test fun helloRoundTrip() {
        val bytes = FrameCodec.hello(HelloInfo(name = "Pixel \"7\"", sampleRate = 48000))
        val frame = FrameCodec.read(ByteArrayInputStream(bytes))
        assertEquals(Protocol.TYPE_HELLO, frame.type)
        val json = Json.parseFlat(String(frame.payload, Charsets.UTF_8))
        assertEquals("Pixel \"7\"", json["name"])
        assertEquals(48000L, json["sampleRate"])
        assertEquals(1L, json["channels"])
        assertEquals("s16le", json["format"])
        assertEquals(10L, json["frameMs"])
        assertEquals(1L, json["version"])
    }

    @Test fun statusJson() {
        val frame = FrameCodec.read(ByteArrayInputStream(FrameCodec.status(StatusInfo(true, 150, "Built-in microphone"))))
        assertEquals(Protocol.TYPE_STATUS, frame.type)
        assertEquals("""{"muted":true,"volume":150,"input":"Built-in microphone"}""", String(frame.payload, Charsets.UTF_8))
    }

    @Test fun audioFrameLayout() {
        val samples = shortArrayOf(1, -2, 0x1234, Short.MIN_VALUE)
        val bytes = FrameCodec.audio(seq = 0xFFFFFFFFL + 6, captureTimeMicros = 0x0102030405060708L, samples = samples)
        val bb = ByteBuffer.wrap(bytes)
        assertEquals(Protocol.TYPE_AUDIO, bb.get())
        assertEquals(12 + 8, bb.int)
        assertEquals(5, bb.int) // seq wraps at u32
        assertEquals(0x0102030405060708L, bb.long)
        // PCM is little endian
        assertArrayEquals(byteArrayOf(1, 0, 0xFE.toByte(), 0xFF.toByte(), 0x34, 0x12, 0, 0x80.toByte()), bytes.copyOfRange(17, 25))

        val frame = FrameCodec.read(ByteArrayInputStream(bytes))
        val audio = FrameCodec.decodeAudio(frame.payload)
        assertEquals(5L, audio.seq)
        assertEquals(0x0102030405060708L, audio.captureTimeMicros)
        assertArrayEquals(samples, audio.samples)
    }

    @Test fun audioWithCountEncodesPrefixOnly() {
        val audio = FrameCodec.decodeAudio(FrameCodec.read(ByteArrayInputStream(FrameCodec.audio(1, 2, shortArrayOf(7, 8, 9), 2))).payload)
        assertArrayEquals(shortArrayOf(7, 8), audio.samples)
    }

    @Test fun multipleFramesFromStream() {
        val out = ByteArrayOutputStream()
        out.write(FrameCodec.hello(HelloInfo("a", 44100)))
        out.write(FrameCodec.status(StatusInfo(false, 100, "x")))
        out.write(FrameCodec.audio(0, 0, ShortArray(441)))
        val input = ByteArrayInputStream(out.toByteArray())
        assertEquals(Protocol.TYPE_HELLO, FrameCodec.read(input).type)
        assertEquals(Protocol.TYPE_STATUS, FrameCodec.read(input).type)
        assertEquals(441, FrameCodec.decodeAudio(FrameCodec.read(input).payload).samples.size)
        try { FrameCodec.read(input); throw AssertionError("expected EOF") } catch (_: EOFException) {}
    }

    @Test(expected = ProtocolException::class)
    fun oversizedPayloadRejected() {
        val bytes = byteArrayOf(0x02) + ByteBuffer.allocate(4).putInt(Protocol.MAX_PAYLOAD + 1).array()
        FrameCodec.read(ByteArrayInputStream(bytes))
    }

    @Test(expected = ProtocolException::class)
    fun shortAudioPayloadRejected() {
        FrameCodec.decodeAudio(ByteArray(11))
    }

    @Test fun discoveryRequestAndReply() {
        assertTrue(Discovery.isRequest("VPMIC_DISCOVER_V1".toByteArray()))
        assertTrue(Discovery.isRequest("VPMIC_DISCOVER_V1\n".toByteArray()))
        assertFalse(Discovery.isRequest("VPMIC_DISCOVER_V2".toByteArray()))
        assertFalse(Discovery.isRequest("hello".toByteArray()))
        assertNull(Discovery.replyFor("nope".toByteArray(), 4, "x"))
        val reply = Discovery.replyFor(Protocol.DISCOVER_REQUEST, Protocol.DISCOVER_REQUEST.size, "My Phone")!!
        assertEquals("""{"service":"vibe-phone-mic","version":1,"name":"My Phone","port":48777}""", String(reply, Charsets.UTF_8))
    }

    @Test fun jsonEscapingRoundTrips() {
        val tricky = "a\"b\\c\nd\u0001é🎤"
        val parsed = Json.parseFlat(Json.obj("k" to tricky, "n" to -3, "b" to false, "z" to null))
        assertEquals(tricky, parsed["k"])
        assertEquals(-3L, parsed["n"])
        assertEquals(false, parsed["b"])
        assertTrue(parsed.containsKey("z"))
    }
}
