package com.vibephonemic.core

import java.io.DataInputStream
import java.io.IOException
import java.net.InetSocketAddress
import java.net.ServerSocket
import java.net.Socket
import java.net.SocketException
import java.util.concurrent.ArrayBlockingQueue
import java.util.concurrent.CountDownLatch
import java.util.concurrent.TimeUnit
import java.util.concurrent.atomic.AtomicReference

/**
 * Platform-independent TCP audio server implementing wire protocol v1.
 *
 * Owns a capture thread that continuously reads from an [AudioSource], applies gain / mute and
 * forwards AUDIO frames to the single active stream client. Probe clients get HELLO and are closed.
 */
class MicServer(
    private val port: Int = Protocol.PORT,
    deviceName: String,
    private val initialSource: AudioSource,
    private val listener: Listener = object : Listener {},
    private val fallbackSource: (sampleRate: Int) -> AudioSource = { SilenceAudioSource(it) },
    private val watchdogFactory: () -> CaptureWatchdog = { CaptureWatchdog() },
) {
    interface Listener {
        /** Called with the client's IP when a stream client connects, or null when it disconnects. */
        fun onClientChanged(address: String?) {}

        /** Post-gain input level 0..1, roughly 30 times per second. */
        fun onLevel(level: Float) {}

        /** Called when the capture source has been (re)opened. [error] is set if a fallback was used. */
        fun onSourceChanged(sampleRate: Int, description: String, error: String?) {}

        /** Diagnostic messages (e.g. the capture watchdog reopening a wedged microphone). */
        fun onWarning(message: String) {}
    }

    private val lock = Any()
    @Volatile private var running = false
    private var serverSocket: ServerSocket? = null
    private var acceptThread: Thread? = null
    private var captureThread: Thread? = null
    private val pendingSource = AtomicReference<AudioSource?>(null)

    @Volatile var deviceName: String = deviceName
    @Volatile private var muted = false
    @Volatile private var volume = 100
    @Volatile private var sampleRate = 48000
    @Volatile private var inputDescription = initialSource.description
    private var streamClient: Client? = null

    // Monotonic clock anchored to wall time, in microseconds.
    private val wallBaseMicros = System.currentTimeMillis() * 1000
    private val nanoBase = System.nanoTime()
    private fun nowMicros(): Long = wallBaseMicros + (System.nanoTime() - nanoBase) / 1000

    val localPort: Int get() = serverSocket?.localPort ?: -1
    val currentSampleRate: Int get() = sampleRate
    val isRunning: Boolean get() = running
    val clientAddress: String? get() = synchronized(lock) { streamClient?.address }

    fun status(): StatusInfo = StatusInfo(muted, volume, inputDescription)
    fun hello(): HelloInfo = HelloInfo(name = deviceName, sampleRate = sampleRate)

    @Synchronized
    fun start() {
        if (running) return
        val ss = ServerSocket()
        ss.reuseAddress = true
        ss.bind(InetSocketAddress(port))
        serverSocket = ss
        running = true
        val source = openSource(initialSource)
        captureThread = Thread({ captureLoop(source) }, "vpm-capture").apply { isDaemon = true; start() }
        acceptThread = Thread({ acceptLoop(ss) }, "vpm-accept").apply { isDaemon = true; start() }
    }

    @Synchronized
    fun stop() {
        if (!running) return
        running = false
        try { serverSocket?.close() } catch (_: IOException) {}
        val c = synchronized(lock) { streamClient.also { streamClient = null } }
        if (c != null) {
            c.closeWithBye(Protocol.BYE_STOPPED)
            c.awaitClosed(1500)
            listener.onClientChanged(null)
        }
        captureThread?.join(2000)
        acceptThread?.join(2000)
        captureThread = null
        acceptThread = null
        serverSocket = null
    }

    fun setMuted(value: Boolean) = updateStatus { muted = value }

    fun setVolume(percent: Int) = updateStatus { volume = AudioMath.clampVolume(percent) }

    /** Replaces the capture source (e.g. a different microphone). Takes effect within one frame. */
    fun setAudioSource(source: AudioSource) {
        pendingSource.set(source)
    }

    private inline fun updateStatus(change: () -> Unit) {
        synchronized(lock) {
            val before = status()
            change()
            val after = status()
            if (after != before) streamClient?.enqueue(FrameCodec.status(after))
        }
    }

    private fun openSource(source: AudioSource): AudioSource {
        val opened = try {
            source.start()
            listener.onSourceChanged(source.sampleRate, source.description, null)
            source
        } catch (e: Exception) {
            val fb = fallbackSource(48000)
            fb.start()
            listener.onSourceChanged(fb.sampleRate, fb.description, e.message ?: e.toString())
            fb
        }
        sampleRate = opened.sampleRate
        updateStatus { inputDescription = opened.description }
        return opened
    }

    private fun frameSamples(rate: Int) = rate * Protocol.FRAME_MS / 1000

    private fun captureLoop(initial: AudioSource) {
        var src = initial
        var buf = ShortArray(frameSamples(src.sampleRate))
        var levelPeak = 0f
        var levelFrames = 0
        var watchdog = watchdogFactory()

        fun switchTo(next: AudioSource, restartSame: Boolean) {
            val oldRate = src.sampleRate
            try { src.stop() } catch (_: Exception) {}
            src = if (restartSame) reopen(next) else openSource(next)
            buf = ShortArray(frameSamples(src.sampleRate))
            // Sample rate change invalidates the HELLO the client got: make it reconnect.
            if (src.sampleRate != oldRate) dropStreamClient()
        }

        try {
            while (running) {
                val next = pendingSource.getAndSet(null)
                if (next != null) {
                    switchTo(next, restartSame = false)
                    watchdog = watchdogFactory()
                }
                val n = try { src.read(buf, buf.size) } catch (e: Exception) { -1 }
                if (n <= 0) {
                    when (watchdog.onReadError()) {
                        CaptureWatchdog.Action.REOPEN -> {
                            listener.onWarning("Capture read failed repeatedly; reopening ${src.description} (attempt ${watchdog.consecutiveReopens})")
                            switchTo(src, restartSame = true)
                        }
                        CaptureWatchdog.Action.GIVE_UP -> {
                            listener.onWarning("Capture keeps failing; switching to silence")
                            pendingSource.compareAndSet(null, fallbackSource(src.sampleRate))
                        }
                        CaptureWatchdog.Action.OK -> {}
                    }
                    Thread.sleep(10)
                    continue
                }
                when (watchdog.onFrame(buf, n, monitored = src.watchdogEnabled && !muted)) {
                    CaptureWatchdog.Action.REOPEN -> {
                        listener.onWarning("1 s of digital silence while unmuted; reopening ${src.description} (attempt ${watchdog.consecutiveReopens})")
                        switchTo(src, restartSame = true)
                    }
                    CaptureWatchdog.Action.GIVE_UP ->
                        listener.onWarning("Microphone still delivers digital silence after reopening; giving up")
                    CaptureWatchdog.Action.OK -> {}
                }
                AudioMath.applyGain(buf, n, volume, muted)
                levelPeak = maxOf(levelPeak, AudioMath.meterLevel(buf, n))
                if (++levelFrames >= 3) {
                    listener.onLevel(levelPeak)
                    levelPeak = 0f
                    levelFrames = 0
                }
                val c = synchronized(lock) { streamClient }
                if (c != null) c.enqueue(FrameCodec.audio(c.nextSeq++, nowMicros(), buf, n.coerceAtMost(buf.size)))
            }
        } catch (_: InterruptedException) {
        } finally {
            try { src.stop() } catch (_: Exception) {}
            listener.onLevel(0f)
        }
    }

    /** Restarts the same source after a wedge. A failed start leaves it closed so reads error out and retry. */
    private fun reopen(source: AudioSource): AudioSource {
        try {
            source.start()
            sampleRate = source.sampleRate
        } catch (e: Exception) {
            listener.onWarning("Reopening ${source.description} failed: ${e.message}")
        }
        return source
    }

    private fun dropStreamClient() {
        val c = synchronized(lock) { streamClient }
        c?.close()
    }

    private fun acceptLoop(ss: ServerSocket) {
        while (running) {
            val socket = try { ss.accept() } catch (e: IOException) { if (running) continue else break }
            Thread({ handleConnection(socket) }, "vpm-conn").apply { isDaemon = true; start() }
        }
    }

    private fun handleConnection(socket: Socket) {
        try {
            socket.tcpNoDelay = true
            socket.soTimeout = 3000
            val hs = ByteArray(Handshake.SIZE)
            DataInputStream(socket.getInputStream()).readFully(hs)
            val mode = Handshake.parse(hs)
            if (mode == null || !running) { socket.close(); return }
            socket.soTimeout = 0
            when (mode) {
                ClientMode.PROBE -> {
                    socket.getOutputStream().apply { write(FrameCodec.hello(hello())); flush() }
                    socket.close()
                }
                ClientMode.STREAM -> {
                    val client = Client(socket)
                    val old: Client?
                    synchronized(lock) {
                        client.enqueue(FrameCodec.hello(hello()))
                        client.enqueue(FrameCodec.status(status()))
                        old = streamClient
                        streamClient = client
                    }
                    old?.closeWithBye(Protocol.BYE_REPLACED)
                    // Notify before the I/O threads start so a fast disconnect can't be reported first.
                    listener.onClientChanged(client.address)
                    client.startThreads()
                }
            }
        } catch (_: IOException) {
            try { socket.close() } catch (_: IOException) {}
        }
    }

    private fun onClientClosed(c: Client) {
        val wasCurrent = synchronized(lock) {
            if (streamClient === c) { streamClient = null; true } else false
        }
        if (wasCurrent) listener.onClientChanged(null)
    }

    private inner class Client(private val socket: Socket) {
        val address: String = socket.inetAddress?.hostAddress ?: "?"
        private val queue = ArrayBlockingQueue<ByteArray>(64)
        @Volatile private var closed = false
        @Volatile private var closing = false
        private val closedLatch = CountDownLatch(1)
        /** Only touched by the capture thread. */
        var nextSeq = 0L

        fun enqueue(frame: ByteArray) {
            if (closed || closing) return
            while (!queue.offer(frame)) queue.poll() // slow client: drop oldest
        }

        fun startThreads() {
            Thread({ writeLoop() }, "vpm-writer").apply { isDaemon = true; start() }
            Thread({ readLoop() }, "vpm-reader").apply { isDaemon = true; start() }
        }

        private fun writeLoop() {
            try {
                val out = socket.getOutputStream().buffered(64 * 1024)
                while (!closed) {
                    val f = queue.poll(200, TimeUnit.MILLISECONDS) ?: continue
                    out.write(f)
                    var more = queue.poll()
                    while (more != null) { out.write(more); more = queue.poll() }
                    out.flush()
                    if (closing && queue.isEmpty()) break
                }
            } catch (_: IOException) {
            } catch (_: InterruptedException) {
            } finally {
                close()
            }
        }

        private fun readLoop() {
            try {
                val input = socket.getInputStream()
                val sink = ByteArray(256)
                while (!closed && input.read(sink) >= 0) { /* ignore client data */ }
            } catch (_: SocketException) {
            } catch (_: IOException) {
            } finally {
                close()
            }
        }

        /**
         * Sends BYE {reason} after any frames already queued are dropped, then closes. Never blocks;
         * a stalled client is force-closed after a short grace period.
         */
        fun closeWithBye(reason: String) {
            synchronized(this) {
                if (closed || closing) return
                closing = true
                queue.clear()
                queue.offer(FrameCodec.bye(reason))
            }
            Thread({
                if (!closedLatch.await(1000, TimeUnit.MILLISECONDS)) close()
            }, "vpm-bye-timeout").apply { isDaemon = true; start() }
        }

        fun awaitClosed(timeoutMs: Long) = closedLatch.await(timeoutMs, TimeUnit.MILLISECONDS)

        fun close() {
            synchronized(this) {
                if (closed) return
                closed = true
            }
            try { socket.close() } catch (_: IOException) {}
            queue.clear()
            closedLatch.countDown()
            onClientClosed(this)
        }
    }
}
