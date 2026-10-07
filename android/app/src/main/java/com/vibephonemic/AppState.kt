package com.vibephonemic

import android.content.Context
import kotlinx.coroutines.flow.MutableStateFlow
import kotlinx.coroutines.flow.StateFlow
import kotlinx.coroutines.flow.asStateFlow
import kotlinx.coroutines.flow.update

/** Persisted user settings. [inputDeviceId] 0 = system default microphone. */
data class UserSettings(
    val muted: Boolean = false,
    val volume: Int = 100,
    val inputDeviceId: Int = 0,
    val inputDeviceType: Int = 0,
    val inputDeviceAddress: String = "",
)

/** Live server state published by [MicService] and rendered by the UI. */
data class ServerState(
    val running: Boolean = false,
    val clientAddress: String? = null,
    val sampleRate: Int = 0,
    val inputDescription: String = "",
    val error: String? = null,
)

/** Process-wide state shared between the activity and the foreground service. */
object AppState {
    private const val PREFS = "vpm_settings"

    private val _settings = MutableStateFlow(UserSettings())
    val settings: StateFlow<UserSettings> = _settings.asStateFlow()

    val server = MutableStateFlow(ServerState())
    val level = MutableStateFlow(0f)

    /** Debug-only: frequency of the generated test tone that replaces the microphone, or null. */
    @Volatile var testToneHz: Int? = null

    /** Set when the user stops the server explicitly so reopening the activity doesn't restart it. */
    @Volatile var userStopped = false

    private var loaded = false

    /** Forgets in-memory state so the next [load] re-reads disk (simulates a process restart; tests only). */
    @Synchronized
    internal fun resetForTest() {
        loaded = false
        _settings.value = UserSettings()
        server.value = ServerState()
        level.value = 0f
        testToneHz = null
        userStopped = false
    }

    @Synchronized
    fun load(context: Context) {
        if (loaded) return
        val p = context.getSharedPreferences(PREFS, Context.MODE_PRIVATE)
        _settings.value = UserSettings(
            muted = p.getBoolean("muted", false),
            volume = p.getInt("volume", 100).coerceIn(0, 200),
            inputDeviceId = p.getInt("input_id", 0),
            inputDeviceType = p.getInt("input_type", 0),
            inputDeviceAddress = p.getString("input_address", "") ?: "",
        )
        loaded = true
    }

    fun update(context: Context, transform: (UserSettings) -> UserSettings) {
        load(context)
        _settings.update(transform)
        val s = _settings.value
        context.getSharedPreferences(PREFS, Context.MODE_PRIVATE).edit()
            .putBoolean("muted", s.muted)
            .putInt("volume", s.volume)
            .putInt("input_id", s.inputDeviceId)
            .putInt("input_type", s.inputDeviceType)
            .putString("input_address", s.inputDeviceAddress)
            .apply()
    }
}
