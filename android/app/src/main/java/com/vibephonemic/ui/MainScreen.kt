package com.vibephonemic.ui

import androidx.compose.foundation.Canvas
import androidx.compose.foundation.background
import androidx.compose.foundation.border
import androidx.compose.foundation.interaction.MutableInteractionSource
import androidx.compose.foundation.layout.Box
import androidx.compose.foundation.layout.PaddingValues
import androidx.compose.foundation.layout.size
import androidx.compose.foundation.shape.CircleShape
import androidx.compose.material3.ExperimentalMaterial3Api
import androidx.compose.runtime.remember
import androidx.compose.ui.draw.shadow
import androidx.compose.ui.geometry.CornerRadius
import androidx.compose.ui.geometry.Size
import androidx.compose.foundation.layout.Arrangement
import androidx.compose.foundation.layout.Column
import androidx.compose.foundation.layout.Row
import androidx.compose.foundation.layout.Spacer
import androidx.compose.foundation.layout.WindowInsets
import androidx.compose.foundation.layout.fillMaxSize
import androidx.compose.foundation.layout.fillMaxWidth
import androidx.compose.foundation.layout.height
import androidx.compose.foundation.layout.padding
import androidx.compose.foundation.layout.safeDrawing
import androidx.compose.foundation.layout.width
import androidx.compose.foundation.layout.windowInsetsPadding
import androidx.compose.foundation.rememberScrollState
import androidx.compose.foundation.shape.RoundedCornerShape
import androidx.compose.foundation.text.selection.SelectionContainer
import androidx.compose.foundation.verticalScroll
import androidx.compose.material.icons.Icons
import androidx.compose.material.icons.filled.Settings
import androidx.compose.material3.Button
import androidx.compose.material3.ButtonDefaults
import androidx.compose.material3.Card
import androidx.compose.material3.CardDefaults
import androidx.compose.material3.Icon
import androidx.compose.material3.IconButton
import androidx.compose.material3.MaterialTheme
import androidx.compose.material3.OutlinedButton
import androidx.compose.material3.Slider
import androidx.compose.material3.Surface
import androidx.compose.material3.Text
import androidx.compose.material3.TextButton
import androidx.compose.runtime.Composable
import androidx.compose.ui.Alignment
import androidx.compose.ui.Modifier
import androidx.compose.ui.platform.testTag
import androidx.compose.ui.semantics.contentDescription
import androidx.compose.ui.semantics.semantics
import androidx.compose.ui.text.font.FontFamily
import androidx.compose.ui.text.font.FontWeight
import androidx.compose.ui.text.style.TextAlign
import androidx.compose.ui.unit.dp
import androidx.compose.ui.unit.sp
import com.vibephonemic.LocalAddress
import com.vibephonemic.ServerState
import com.vibephonemic.core.AudioMath
import com.vibephonemic.core.Protocol

data class MainUiState(
    val permissionGranted: Boolean = true,
    val addresses: List<LocalAddress> = emptyList(),
    val port: Int = Protocol.PORT,
    val server: ServerState = ServerState(),
    val muted: Boolean = false,
    val volume: Int = 100,
)

@Composable
fun MainScreen(
    state: MainUiState,
    level: Float,
    onToggleMute: () -> Unit,
    onVolumeChange: (Int) -> Unit,
    onStartStop: () -> Unit,
    onGrantPermission: () -> Unit,
    onOpenSettings: () -> Unit,
) {
    Surface(Modifier.fillMaxSize(), color = MaterialTheme.colorScheme.background) {
        Column(
            Modifier
                .fillMaxSize()
                .windowInsetsPadding(WindowInsets.safeDrawing)
                .verticalScroll(rememberScrollState())
                .padding(horizontal = 20.dp, vertical = 8.dp),
            verticalArrangement = Arrangement.spacedBy(12.dp),
        ) {
            Header(onOpenSettings)
            if (!state.permissionGranted) PermissionCard(onGrantPermission)
            AddressCard(state)
            StatusCard(state.server, state.permissionGranted, onStartStop)
            MicControls(state, level, onToggleMute, onVolumeChange)
        }
    }
}

@Composable
private fun Header(onOpenSettings: () -> Unit) {
    Row(verticalAlignment = Alignment.CenterVertically) {
        Logo()
        Spacer(Modifier.width(12.dp))
        Text("Vibe Phone Mic", style = MaterialTheme.typography.titleLarge, fontWeight = FontWeight.SemiBold, modifier = Modifier.weight(1f))
        IconButton(
            onClick = onOpenSettings,
            modifier = Modifier.testTag("settings_button"),
            interactionSource = rememberSoundInteraction(ButtonSound.CarsonMicroflip),
        ) {
            Icon(Icons.Filled.Settings, contentDescription = "Settings")
        }
    }
}

@Composable
private fun SectionCard(modifier: Modifier = Modifier, content: @Composable () -> Unit) {
    Card(
        modifier = modifier.fillMaxWidth(),
        shape = RoundedCornerShape(20.dp),
        colors = CardDefaults.cardColors(containerColor = MaterialTheme.colorScheme.surfaceContainer),
    ) {
        Column(Modifier.padding(horizontal = 18.dp, vertical = 14.dp), verticalArrangement = Arrangement.spacedBy(6.dp)) { content() }
    }
}

@Composable
private fun PermissionCard(onGrant: () -> Unit) {
    SectionCard(Modifier.testTag("permission_card")) {
        Text("Microphone permission needed", style = MaterialTheme.typography.titleMedium)
        Text(
            "Vibe Phone Mic streams your microphone to your computer. Grant access to start the server.",
            style = MaterialTheme.typography.bodyMedium,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
        )
        Button(
            onClick = onGrant,
            modifier = Modifier.testTag("grant_permission"),
            interactionSource = rememberSoundInteraction(ButtonSound.KvmSwitch),
        ) { Text("Grant access") }
    }
}

@Composable
private fun AddressCard(state: MainUiState) {
    SectionCard {
        Text("Connect your computer to", style = MaterialTheme.typography.labelLarge, color = MaterialTheme.colorScheme.onSurfaceVariant)
        if (state.addresses.isEmpty()) {
            Text(
                "No network connection.\nJoin Wi-Fi or use USB below.",
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.testTag("no_network_text"),
            )
        }
        SelectionContainer {
            Column(verticalArrangement = Arrangement.spacedBy(6.dp)) {
                // One address gets the big treatment; several are a little smaller so all fits on screen.
                val many = state.addresses.size >= 2
                state.addresses.forEach { addr ->
                    Column {
                        Text(addr.label, style = MaterialTheme.typography.labelMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
                        Text(
                            addr.ip,
                            fontSize = if (many) IP_SIZE_MANY else IP_SIZE_SINGLE,
                            lineHeight = if (many) 32.sp else 40.sp,
                            fontWeight = FontWeight.SemiBold,
                            fontFamily = FontFamily.Monospace,
                            modifier = Modifier.testTag("ip_text"),
                        )
                    }
                }
            }
        }
        Row(verticalAlignment = Alignment.Bottom) {
            Text("Port ", style = MaterialTheme.typography.titleMedium, color = MaterialTheme.colorScheme.onSurfaceVariant)
            Text(
                state.port.toString(),
                fontSize = 28.sp,
                fontWeight = FontWeight.SemiBold,
                fontFamily = FontFamily.Monospace,
                modifier = Modifier.testTag("port_text"),
            )
        }
        Spacer(Modifier.height(4.dp))
        Text(
            "USB: run  adb forward tcp:${state.port} tcp:${state.port}  on the computer, then connect to 127.0.0.1",
            style = MaterialTheme.typography.bodySmall,
            color = MaterialTheme.colorScheme.onSurfaceVariant,
            modifier = Modifier.testTag("usb_hint"),
        )
    }
}

val IP_SIZE_SINGLE = 34.sp
val IP_SIZE_MANY = 26.sp

@Composable
private fun StatusCard(server: ServerState, permissionGranted: Boolean, onStartStop: () -> Unit) {
    SectionCard {
        Row(verticalAlignment = Alignment.CenterVertically) {
            val connected = server.clientAddress != null
            StatusDot(active = connected)
            val text = when {
                !server.running -> "Server stopped"
                connected -> "Connected to ${server.clientAddress}"
                else -> "Waiting for computer…"
            }
            Text(
                text,
                style = MaterialTheme.typography.titleMedium,
                modifier = Modifier.weight(1f).testTag("connection_status"),
            )
            OutlinedButton(
                onClick = onStartStop,
                enabled = permissionGranted,
                contentPadding = PaddingValues(horizontal = 14.dp, vertical = 0.dp),
                modifier = Modifier.height(34.dp).testTag("server_toggle"),
                colors = ButtonDefaults.outlinedButtonColors(contentColor = MaterialTheme.colorScheme.onSurface),
                interactionSource = rememberSoundInteraction(ButtonSound.KvmSwitch),
            ) {
                Text(if (server.running) "Stop" else "Start", style = MaterialTheme.typography.labelLarge)
            }
        }
        if (server.running && server.sampleRate > 0) {
            Text(
                "${server.sampleRate} Hz mono · ${server.inputDescription}",
                style = MaterialTheme.typography.bodySmall,
                color = MaterialTheme.colorScheme.onSurfaceVariant,
                modifier = Modifier.testTag("audio_info"),
            )
        }
        server.error?.let {
            Text(it, style = MaterialTheme.typography.bodySmall, color = MaterialTheme.colorScheme.error, modifier = Modifier.testTag("error_text"))
        }
    }
}

@Composable
private fun MicControls(state: MainUiState, level: Float, onToggleMute: () -> Unit, onVolumeChange: (Int) -> Unit) {
    val colors = MaterialTheme.colorScheme
    SectionCard {
        Column(Modifier.fillMaxWidth(), horizontalAlignment = Alignment.CenterHorizontally) {
            Spacer(Modifier.height(0.dp))
            MuteButton(muted = state.muted, onToggle = onToggleMute)
            Spacer(Modifier.height(6.dp))
            Text(
                if (state.muted) "Muted — tap to unmute" else "Live — tap to mute",
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.Center,
                modifier = Modifier.testTag("mute_state"),
            )
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            Text("Level", style = MaterialTheme.typography.labelLarge, color = colors.onSurfaceVariant, modifier = Modifier.width(64.dp))
            LevelMeter(level, Modifier.weight(1f))
        }
        Row(verticalAlignment = Alignment.CenterVertically) {
            TextButton(
                onClick = { onVolumeChange(AudioMath.clampVolume(state.volume - 10)) },
                modifier = Modifier.testTag("volume_down").semantics { contentDescription = "Volume down" },
                colors = ButtonDefaults.textButtonColors(contentColor = colors.onSurface),
                interactionSource = rememberSoundInteraction(ButtonSound.CarsonMicroflip),
            ) { Text("−", fontSize = 22.sp) }
            VolumeSlider(
                volume = state.volume,
                onVolumeChange = onVolumeChange,
                modifier = Modifier.weight(1f),
            )
            TextButton(
                onClick = { onVolumeChange(AudioMath.clampVolume(state.volume + 10)) },
                modifier = Modifier.testTag("volume_up").semantics { contentDescription = "Volume up" },
                colors = ButtonDefaults.textButtonColors(contentColor = colors.onSurface),
                interactionSource = rememberSoundInteraction(ButtonSound.CarsonMicroflip),
            ) { Text("+", fontSize = 22.sp) }
            Text(
                AudioMath.percentLabel(state.volume),
                style = MaterialTheme.typography.titleMedium,
                textAlign = TextAlign.End,
                modifier = Modifier.width(60.dp).testTag("volume_text"),
            )
        }
    }
}

/** Light, neutral slider: thin rounded track and a small round thumb. */
@OptIn(ExperimentalMaterial3Api::class)
@Composable
private fun VolumeSlider(volume: Int, onVolumeChange: (Int) -> Unit, modifier: Modifier = Modifier) {
    val colors = MaterialTheme.colorScheme
    val interaction = remember { MutableInteractionSource() }
    Slider(
        value = volume.toFloat(),
        onValueChange = { onVolumeChange(AudioMath.sliderToVolume(it)) },
        valueRange = AudioMath.MIN_VOLUME.toFloat()..AudioMath.MAX_VOLUME.toFloat(),
        interactionSource = interaction,
        thumb = {
            Box(
                Modifier
                    .size(20.dp)
                    .shadow(2.dp, CircleShape)
                    .background(colors.surfaceContainerLowest, CircleShape)
                    .border(1.5.dp, colors.onSurfaceVariant, CircleShape),
            )
        },
        track = { sliderState ->
            val range = sliderState.valueRange
            val fraction = ((sliderState.value - range.start) / (range.endInclusive - range.start)).coerceIn(0f, 1f)
            Canvas(Modifier.fillMaxWidth().height(4.dp)) {
                val r = CornerRadius(size.height / 2, size.height / 2)
                drawRoundRect(colors.outlineVariant, cornerRadius = r)
                drawRoundRect(colors.onSurfaceVariant, size = Size(size.width * fraction, size.height), cornerRadius = r)
            }
        },
        modifier = modifier.testTag("volume_slider").semantics { contentDescription = "Volume" },
    )
}
