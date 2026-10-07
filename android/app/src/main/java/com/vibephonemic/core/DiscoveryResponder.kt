package com.vibephonemic.core

import java.io.IOException
import java.net.DatagramPacket
import java.net.DatagramSocket
import java.net.InetSocketAddress

/** Answers `VPMIC_DISCOVER_V1` UDP datagrams with the service JSON (unicast to the sender). */
class DiscoveryResponder(
    private val bindPort: Int = Protocol.PORT,
    private val advertisedPort: Int = Protocol.PORT,
    private val nameProvider: () -> String,
) {
    @Volatile private var socket: DatagramSocket? = null
    private var thread: Thread? = null

    val localPort: Int get() = socket?.localPort ?: -1

    @Synchronized
    fun start() {
        if (socket != null) return
        val s = DatagramSocket(null)
        s.reuseAddress = true
        s.broadcast = true
        s.bind(InetSocketAddress(bindPort))
        socket = s
        thread = Thread({ loop(s) }, "vpm-discovery").apply { isDaemon = true; start() }
    }

    @Synchronized
    fun stop() {
        socket?.close()
        socket = null
        thread?.join(1000)
        thread = null
    }

    private fun loop(s: DatagramSocket) {
        val buf = ByteArray(512)
        while (!s.isClosed) {
            try {
                val packet = DatagramPacket(buf, buf.size)
                s.receive(packet)
                val reply = Discovery.replyFor(packet.data, packet.length, nameProvider(), advertisedPort) ?: continue
                s.send(DatagramPacket(reply, reply.size, packet.socketAddress))
            } catch (_: IOException) {
                if (s.isClosed) break
            }
        }
    }
}
