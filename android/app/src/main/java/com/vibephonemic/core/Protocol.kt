package com.vibephonemic.core

import java.io.DataInputStream
import java.io.EOFException
import java.io.IOException
import java.io.InputStream
import java.nio.ByteBuffer
import java.nio.ByteOrder

/** Wire protocol v1 constants and codecs (see PLAN.md). All integers big-endian. */
object Protocol {
    const val PORT = 48777
    const val VERSION = 1
    const val SERVICE = "vibe-phone-mic"
    const val FRAME_MS = 10
    const val MAX_PAYLOAD = 1 shl 20

    const val TYPE_HELLO: Byte = 0x01
    const val TYPE_AUDIO: Byte = 0x02
    const val TYPE_STATUS: Byte = 0x03
    const val TYPE_BYE: Byte = 0x04

    const val BYE_REPLACED = "replaced"
    const val BYE_STOPPED = "stopped"

    const val HEADER_SIZE = 5
    const val AUDIO_PREFIX_SIZE = 12

    val DISCOVER_REQUEST: ByteArray = "VPMIC_DISCOVER_V1".toByteArray(Charsets.US_ASCII)
}

class ProtocolException(message: String) : IOException(message)

enum class ClientMode(val code: Int) { STREAM(0), PROBE(1) }

object Handshake {
    const val SIZE = 8
    private val MAGIC = "VPMC".toByteArray(Charsets.US_ASCII)

    fun encode(mode: ClientMode, version: Int = Protocol.VERSION): ByteArray =
        byteArrayOf(MAGIC[0], MAGIC[1], MAGIC[2], MAGIC[3], version.toByte(), mode.code.toByte(), 0, 0)

    /** Returns the requested mode, or null if the handshake is invalid. Reserved bytes are ignored. */
    fun parse(bytes: ByteArray): ClientMode? {
        if (bytes.size != SIZE) return null
        for (i in 0 until 4) if (bytes[i] != MAGIC[i]) return null
        if (bytes[4].toInt() != Protocol.VERSION) return null
        return when (bytes[5].toInt()) {
            0 -> ClientMode.STREAM
            1 -> ClientMode.PROBE
            else -> null
        }
    }
}

data class Frame(val type: Byte, val payload: ByteArray) {
    override fun equals(other: Any?): Boolean =
        other is Frame && other.type == type && other.payload.contentEquals(payload)

    override fun hashCode(): Int = 31 * type + payload.contentHashCode()
}

data class AudioPayload(val seq: Long, val captureTimeMicros: Long, val samples: ShortArray)

data class HelloInfo(
    val name: String,
    val sampleRate: Int,
    val channels: Int = 1,
    val format: String = "s16le",
    val frameMs: Int = Protocol.FRAME_MS,
    val version: Int = Protocol.VERSION,
) {
    fun toJson(): String = Json.obj(
        "name" to name,
        "sampleRate" to sampleRate,
        "channels" to channels,
        "format" to format,
        "frameMs" to frameMs,
        "version" to version,
    )
}

data class StatusInfo(val muted: Boolean, val volume: Int, val input: String) {
    fun toJson(): String = Json.obj("muted" to muted, "volume" to volume, "input" to input)
}

object FrameCodec {
    fun encode(type: Byte, payload: ByteArray): ByteArray {
        require(payload.size <= Protocol.MAX_PAYLOAD) { "payload too large" }
        val buf = ByteBuffer.allocate(Protocol.HEADER_SIZE + payload.size).order(ByteOrder.BIG_ENDIAN)
        buf.put(type).putInt(payload.size).put(payload)
        return buf.array()
    }

    fun hello(info: HelloInfo): ByteArray = encode(Protocol.TYPE_HELLO, info.toJson().toByteArray(Charsets.UTF_8))

    fun status(info: StatusInfo): ByteArray = encode(Protocol.TYPE_STATUS, info.toJson().toByteArray(Charsets.UTF_8))

    /** BYE: sent right before the phone deliberately closes a stream client. */
    fun bye(reason: String): ByteArray = encode(Protocol.TYPE_BYE, Json.obj("reason" to reason).toByteArray(Charsets.UTF_8))

    /** Encodes an AUDIO frame: u32 seq | u64 captureTimeMicros | PCM s16le. */
    fun audio(seq: Long, captureTimeMicros: Long, samples: ShortArray, count: Int = samples.size): ByteArray {
        val payloadLen = Protocol.AUDIO_PREFIX_SIZE + count * 2
        val buf = ByteBuffer.allocate(Protocol.HEADER_SIZE + payloadLen).order(ByteOrder.BIG_ENDIAN)
        buf.put(Protocol.TYPE_AUDIO).putInt(payloadLen)
        buf.putInt((seq and 0xFFFFFFFFL).toInt()).putLong(captureTimeMicros)
        buf.order(ByteOrder.LITTLE_ENDIAN)
        for (i in 0 until count) buf.putShort(samples[i])
        return buf.array()
    }

    fun decodeAudio(payload: ByteArray): AudioPayload {
        if (payload.size < Protocol.AUDIO_PREFIX_SIZE || (payload.size - Protocol.AUDIO_PREFIX_SIZE) % 2 != 0) {
            throw ProtocolException("bad AUDIO payload length ${payload.size}")
        }
        val buf = ByteBuffer.wrap(payload).order(ByteOrder.BIG_ENDIAN)
        val seq = buf.int.toLong() and 0xFFFFFFFFL
        val ts = buf.long
        buf.order(ByteOrder.LITTLE_ENDIAN)
        val samples = ShortArray((payload.size - Protocol.AUDIO_PREFIX_SIZE) / 2) { buf.short }
        return AudioPayload(seq, ts, samples)
    }

    /** Reads one frame; throws [EOFException] at clean end of stream, [ProtocolException] on bad data. */
    fun read(input: InputStream): Frame {
        val din = if (input is DataInputStream) input else DataInputStream(input)
        val type = din.read()
        if (type < 0) throw EOFException()
        val len = din.readInt()
        if (len < 0 || len > Protocol.MAX_PAYLOAD) throw ProtocolException("payload length $len out of range")
        val payload = ByteArray(len)
        din.readFully(payload)
        return Frame(type.toByte(), payload)
    }
}

object Discovery {
    fun isRequest(data: ByteArray, length: Int = data.size): Boolean {
        val s = String(data, 0, length, Charsets.US_ASCII).trim()
        return s == "VPMIC_DISCOVER_V1"
    }

    fun reply(name: String, port: Int = Protocol.PORT): ByteArray =
        Json.obj("service" to Protocol.SERVICE, "version" to Protocol.VERSION, "name" to name, "port" to port)
            .toByteArray(Charsets.UTF_8)

    /** Pure reply logic: returns the bytes to send back, or null to ignore the datagram. */
    fun replyFor(data: ByteArray, length: Int, name: String, port: Int = Protocol.PORT): ByteArray? =
        if (isRequest(data, length)) reply(name, port) else null
}
