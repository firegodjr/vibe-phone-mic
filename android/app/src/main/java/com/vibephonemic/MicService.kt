package com.vibephonemic

import android.annotation.SuppressLint
import android.app.Notification
import android.app.NotificationChannel
import android.app.NotificationManager
import android.app.PendingIntent
import android.app.Service
import android.content.Context
import android.content.Intent
import android.content.pm.ServiceInfo
import android.media.AudioDeviceInfo
import android.net.wifi.WifiManager
import android.os.Build
import android.os.IBinder
import android.os.PowerManager
import android.provider.Settings
import android.util.Log
import androidx.core.app.NotificationCompat
import androidx.core.app.ServiceCompat
import androidx.core.content.ContextCompat
import com.vibephonemic.core.AudioSource
import com.vibephonemic.core.DiscoveryResponder
import com.vibephonemic.core.MicServer
import com.vibephonemic.core.Protocol
import kotlinx.coroutines.CoroutineScope
import kotlinx.coroutines.Dispatchers
import kotlinx.coroutines.SupervisorJob
import kotlinx.coroutines.cancel
import kotlinx.coroutines.flow.update
import kotlinx.coroutines.launch
import kotlinx.coroutines.withContext

/**
 * Foreground service (type microphone) owning audio capture, the TCP stream server and the UDP
 * discovery responder. All protocol logic lives in [MicServer] / [DiscoveryResponder].
 */
class MicService : Service() {
    companion object {
        private const val TAG = "VibePhoneMic"
        private const val CHANNEL_ID = "vpm_server"
        private const val NOTIFICATION_ID = 1
        const val ACTION_STOP = "com.vibephonemic.STOP"

        fun start(context: Context) {
            AppState.userStopped = false
            ContextCompat.startForegroundService(context, Intent(context, MicService::class.java))
        }

        fun stop(context: Context) {
            AppState.userStopped = true
            context.startService(Intent(context, MicService::class.java).setAction(ACTION_STOP))
        }

        // Test seams (Robolectric service tests): ephemeral ports and a fake capture source.
        @Volatile internal var portOverride: Int? = null
        @Volatile internal var sourceFactoryOverride: ((AudioDeviceInfo?, String) -> AudioSource)? = null
        @Volatile internal var instance: MicService? = null
    }

    internal val serverForTest: MicServer? get() = server
    internal val discoveryForTest: DiscoveryResponder? get() = discovery

    private val scope = CoroutineScope(SupervisorJob() + Dispatchers.Main.immediate)
    private var server: MicServer? = null
    private var discovery: DiscoveryResponder? = null
    private var multicastLock: WifiManager.MulticastLock? = null
    private var wifiLock: WifiManager.WifiLock? = null
    private var wakeLock: PowerManager.WakeLock? = null
    private var activeSettings: UserSettings? = null
    private var activeToneHz: Int? = null
    private var starting = false

    override fun onBind(intent: Intent?): IBinder? = null

    override fun onStartCommand(intent: Intent?, flags: Int, startId: Int): Int {
        if (intent?.action == ACTION_STOP) {
            AppState.userStopped = true
            // We may have been started with startForegroundService() even though we're about to
            // stop (e.g. STOP while not running): Android requires startForeground() anyway or the
            // app crashes with ForegroundServiceDidNotStartInTimeException.
            if (server != null || starting) {
                try {
                    ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), microphoneType())
                } catch (e: Exception) {
                    enterForegroundBriefly()
                }
            } else {
                enterForegroundBriefly()
            }
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }
        AppState.load(this)
        try {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), microphoneType())
        } catch (e: Exception) {
            // e.g. Android 14+ refuses a microphone FGS started from the background.
            Log.e(TAG, "startForeground failed", e)
            enterForegroundBriefly()
            AppState.server.update { it.copy(running = false, error = "Could not start: ${e.message}") }
            stopForegroundCompat()
            stopSelf()
            return START_NOT_STICKY
        }
        val srv = server
        if (srv == null && !starting) {
            startServer()
        } else if (srv != null && activeToneHz != currentToneHz()) {
            srv.setAudioSource(buildSource(AppState.settings.value))
        }
        return START_NOT_STICKY
    }

    private fun microphoneType(): Int =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.R) ServiceInfo.FOREGROUND_SERVICE_TYPE_MICROPHONE else 0

    /** Satisfies the startForeground() contract without needing microphone eligibility, before stopping. */
    private fun enterForegroundBriefly() {
        val type = when {
            Build.VERSION.SDK_INT >= 34 -> ServiceInfo.FOREGROUND_SERVICE_TYPE_SHORT_SERVICE
            else -> microphoneType()
        }
        try {
            ServiceCompat.startForeground(this, NOTIFICATION_ID, buildNotification(), type)
        } catch (e: Exception) {
            Log.w(TAG, "placeholder startForeground failed", e)
        }
    }

    private fun currentToneHz(): Int? = TestToneHook.activeHz()

    private fun deviceName(): String =
        Settings.Global.getString(contentResolver, Settings.Global.DEVICE_NAME)?.takeIf { it.isNotBlank() }
            ?: "${Build.MANUFACTURER} ${Build.MODEL}".trim()

    private fun buildSource(s: UserSettings): AudioSource {
        activeToneHz = currentToneHz()
        TestToneHook.sourceFor(activeToneHz)?.let { return it }
        val device = InputDevices.find(this, s)
        val label = device?.let { InputDevices.label(it) } ?: InputDevices.DEFAULT_LABEL
        sourceFactoryOverride?.let { return it(device, label) }
        return AudioRecordSource(device, label)
    }

    private fun startServer() {
        starting = true
        acquireLocks()
        val settings = AppState.settings.value
        activeSettings = settings
        val listener = object : MicServer.Listener {
            override fun onClientChanged(address: String?) {
                AppState.server.update { it.copy(clientAddress = address) }
            }

            override fun onLevel(level: Float) {
                AppState.level.value = level
            }

            override fun onWarning(message: String) {
                Log.w(TAG, message)
            }

            override fun onSourceChanged(sampleRate: Int, description: String, error: String?) {
                AppState.server.update {
                    it.copy(sampleRate = sampleRate, inputDescription = description, error = error?.let { e -> "Microphone error: $e" })
                }
            }
        }
        val port = portOverride ?: Protocol.PORT
        val srv = MicServer(port, deviceName(), buildSource(settings), listener, silenceGate = AppState.soundGate)
        srv.setMuted(settings.muted)
        srv.setVolume(settings.volume)
        val disc = DiscoveryResponder(port, Protocol.PORT) { srv.deviceName }
        scope.launch {
            val error = withContext(Dispatchers.IO) {
                try {
                    srv.start()
                    try { disc.start() } catch (e: Exception) { Log.w(TAG, "discovery unavailable", e) }
                    null
                } catch (e: Exception) {
                    Log.e(TAG, "server start failed", e)
                    e.message ?: e.toString()
                }
            }
            starting = false
            if (error != null) {
                AppState.server.update { it.copy(running = false, error = "Server failed to start: $error") }
                stopSelf()
                return@launch
            }
            server = srv
            discovery = disc
            // Serving again (whoever started us): a previous explicit Stop no longer applies.
            AppState.userStopped = false
            AppState.server.update { it.copy(running = true, error = null) }
            AppState.settings.collect { applySettings(it) }
        }
    }

    private fun applySettings(s: UserSettings) {
        val srv = server ?: return
        srv.setMuted(s.muted)
        srv.setVolume(s.volume)
        val prev = activeSettings
        activeSettings = s
        if (prev != null && (prev.inputDeviceId != s.inputDeviceId || prev.inputDeviceAddress != s.inputDeviceAddress)) {
            srv.setAudioSource(buildSource(s))
        }
    }

    @SuppressLint("WakelockTimeout")
    private fun acquireLocks() {
        val wifi = applicationContext.getSystemService(WifiManager::class.java)
        multicastLock = wifi?.createMulticastLock("vpm-discovery")?.apply { setReferenceCounted(false); acquire() }
        @Suppress("DEPRECATION")
        val mode = if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.Q) WifiManager.WIFI_MODE_FULL_LOW_LATENCY else WifiManager.WIFI_MODE_FULL_HIGH_PERF
        wifiLock = wifi?.createWifiLock(mode, "vpm-stream")?.apply { setReferenceCounted(false); acquire() }
        wakeLock = getSystemService(PowerManager::class.java)
            ?.newWakeLock(PowerManager.PARTIAL_WAKE_LOCK, "vibephonemic:capture")
            ?.apply { setReferenceCounted(false); acquire() }
    }

    private fun releaseLocks() {
        try { multicastLock?.release() } catch (_: Exception) {}
        try { wifiLock?.release() } catch (_: Exception) {}
        try { wakeLock?.release() } catch (_: Exception) {}
        multicastLock = null; wifiLock = null; wakeLock = null
    }

    override fun onCreate() {
        super.onCreate()
        instance = this
    }

    override fun onDestroy() {
        if (instance === this) instance = null
        scope.cancel()
        val srv = server
        val disc = discovery
        server = null
        discovery = null
        Thread {
            disc?.stop()
            srv?.stop()
        }.start()
        releaseLocks()
        AppState.server.value = ServerState()
        AppState.level.value = 0f
        super.onDestroy()
    }

    private fun stopForegroundCompat() {
        ServiceCompat.stopForeground(this, ServiceCompat.STOP_FOREGROUND_REMOVE)
    }

    private fun buildNotification(): Notification {
        val nm = getSystemService(NotificationManager::class.java)
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.O) {
            nm.createNotificationChannel(
                NotificationChannel(CHANNEL_ID, getString(R.string.channel_name), NotificationManager.IMPORTANCE_LOW)
            )
        }
        val open = PendingIntent.getActivity(
            this, 0, Intent(this, MainActivity::class.java).addFlags(Intent.FLAG_ACTIVITY_SINGLE_TOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        val stop = PendingIntent.getService(
            this, 1, Intent(this, MicService::class.java).setAction(ACTION_STOP),
            PendingIntent.FLAG_IMMUTABLE or PendingIntent.FLAG_UPDATE_CURRENT,
        )
        return NotificationCompat.Builder(this, CHANNEL_ID)
            .setSmallIcon(R.drawable.ic_mic)
            .setContentTitle(getString(R.string.app_name))
            .setContentText(getString(R.string.notification_text, Protocol.PORT))
            .setOngoing(true)
            .setSilent(true)
            .setContentIntent(open)
            .addAction(0, getString(R.string.stop), stop)
            .setForegroundServiceBehavior(NotificationCompat.FOREGROUND_SERVICE_IMMEDIATE)
            .build()
    }
}
