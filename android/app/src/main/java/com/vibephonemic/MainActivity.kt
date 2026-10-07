package com.vibephonemic

import android.Manifest
import android.content.Intent
import android.content.pm.PackageManager
import android.media.AudioDeviceCallback
import android.media.AudioDeviceInfo
import android.media.AudioManager
import android.os.Build
import android.os.Bundle
import androidx.activity.ComponentActivity
import androidx.activity.compose.BackHandler
import androidx.activity.compose.setContent
import androidx.activity.enableEdgeToEdge
import androidx.activity.result.contract.ActivityResultContracts
import androidx.compose.runtime.Composable
import androidx.compose.runtime.DisposableEffect
import androidx.compose.runtime.getValue
import androidx.compose.runtime.mutableStateOf
import androidx.compose.runtime.remember
import androidx.compose.runtime.saveable.rememberSaveable
import androidx.compose.runtime.setValue
import androidx.compose.ui.ExperimentalComposeUiApi
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.LocalContext
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.semantics.testTagsAsResourceId
import androidx.core.content.ContextCompat
import androidx.lifecycle.Lifecycle
import androidx.lifecycle.compose.collectAsStateWithLifecycle
import androidx.lifecycle.lifecycleScope
import androidx.lifecycle.repeatOnLifecycle
import kotlinx.coroutines.launch
import com.vibephonemic.ui.MainScreen
import com.vibephonemic.ui.MainUiState
import com.vibephonemic.ui.SettingsScreen
import com.vibephonemic.ui.VpmTheme
import kotlinx.coroutines.delay

class MainActivity : ComponentActivity() {
    private var permissionGranted by mutableStateOf(false)
    private var addresses by mutableStateOf(emptyList<LocalAddress>())

    private val permissionLauncher =
        registerForActivityResult(ActivityResultContracts.RequestMultiplePermissions()) {
            refreshPermission()
            if (permissionGranted) MicService.start(this)
        }

    private fun requiredPermissions(): Array<String> =
        if (Build.VERSION.SDK_INT >= Build.VERSION_CODES.TIRAMISU) {
            arrayOf(Manifest.permission.RECORD_AUDIO, Manifest.permission.POST_NOTIFICATIONS)
        } else {
            arrayOf(Manifest.permission.RECORD_AUDIO)
        }

    private fun refreshPermission() {
        permissionGranted = ContextCompat.checkSelfPermission(this, Manifest.permission.RECORD_AUDIO) ==
            PackageManager.PERMISSION_GRANTED
    }

    private fun requestPermissions() = permissionLauncher.launch(requiredPermissions())

    override fun onCreate(savedInstanceState: Bundle?) {
        enableEdgeToEdge()
        super.onCreate(savedInstanceState)
        AppState.load(this)
        applyTestIntent(intent)
        refreshPermission()
        if (!permissionGranted && savedInstanceState == null) requestPermissions()
        addresses = NetworkAddresses.list()
        lifecycleScope.launch {
            repeatOnLifecycle(Lifecycle.State.STARTED) {
                while (true) {
                    addresses = NetworkAddresses.list()
                    delay(2000)
                }
            }
        }
        setContent {
            VpmTheme {
                VpmApp(
                    permissionGranted = permissionGranted,
                    addresses = addresses,
                    onGrantPermission = ::requestPermissions,
                )
            }
        }
    }

    override fun onStart() {
        super.onStart()
        refreshPermission()
        if (permissionGranted && !AppState.userStopped) MicService.start(this)
    }

    public override fun onNewIntent(intent: Intent) {
        super.onNewIntent(intent)
        setIntent(intent)
        if (applyTestIntent(intent) && permissionGranted) MicService.start(this)
    }

    /** Returns true if a debug test-tone extra was applied. No-op in release builds. */
    private fun applyTestIntent(intent: Intent?): Boolean = TestToneHook.apply(intent)
}

@OptIn(ExperimentalComposeUiApi::class)
@Composable
private fun VpmApp(permissionGranted: Boolean, addresses: List<LocalAddress>, onGrantPermission: () -> Unit) {
    val context = LocalContext.current
    val settings by AppState.settings.collectAsStateWithLifecycle()
    val server by AppState.server.collectAsStateWithLifecycle()
    val level by AppState.level.collectAsStateWithLifecycle()
    var showSettings by rememberSaveable { mutableStateOf(false) }

    androidx.compose.foundation.layout.Box(Modifier.semantics { testTagsAsResourceId = true }) {
        if (showSettings) {
            BackHandler { showSettings = false }
            var options by remember { mutableStateOf(InputDevices.list(context)) }
            DisposableEffect(Unit) {
                val am = context.getSystemService(AudioManager::class.java)
                val cb = object : AudioDeviceCallback() {
                    override fun onAudioDevicesAdded(addedDevices: Array<out AudioDeviceInfo>?) {
                        options = InputDevices.list(context)
                    }

                    override fun onAudioDevicesRemoved(removedDevices: Array<out AudioDeviceInfo>?) {
                        options = InputDevices.list(context)
                    }
                }
                am?.registerAudioDeviceCallback(cb, null)
                onDispose { am?.unregisterAudioDeviceCallback(cb) }
            }
            SettingsScreen(
                options = options,
                selectedId = InputDevices.resolve(settings, options)?.id ?: 0,
                versionName = BuildConfig.VERSION_NAME,
                onSelect = { opt ->
                    AppState.update(context) {
                        it.copy(
                            inputDeviceId = opt?.id ?: 0,
                            inputDeviceType = opt?.type ?: 0,
                            inputDeviceAddress = opt?.address ?: "",
                        )
                    }
                },
                onBack = { showSettings = false },
            )
        } else {
            MainScreen(
                state = MainUiState(
                    permissionGranted = permissionGranted,
                    addresses = addresses,
                    server = server,
                    muted = settings.muted,
                    volume = settings.volume,
                ),
                level = level,
                onToggleMute = { AppState.update(context) { it.copy(muted = !it.muted) } },
                onVolumeChange = { v -> AppState.update(context) { it.copy(volume = v) } },
                onStartStop = { if (server.running) MicService.stop(context) else MicService.start(context) },
                onGrantPermission = onGrantPermission,
                onOpenSettings = { showSettings = true },
            )
        }
    }
}
