package com.vibephonemic

import android.content.Context
import android.media.AudioDeviceInfo
import android.media.AudioManager

data class InputOption(val id: Int, val type: Int, val address: String, val label: String)

object InputDevices {
    const val DEFAULT_LABEL = "Default microphone"

    fun list(context: Context): List<InputOption> {
        val am = context.getSystemService(AudioManager::class.java) ?: return emptyList()
        return am.getDevices(AudioManager.GET_DEVICES_INPUTS)
            .filter { it.isSource && it.type != AudioDeviceInfo.TYPE_TELEPHONY && it.type != AudioDeviceInfo.TYPE_REMOTE_SUBMIX && it.type != AudioDeviceInfo.TYPE_FM_TUNER }
            .map { InputOption(it.id, it.type, it.address ?: "", label(it)) }
    }

    /**
     * Pure selection logic: resolves the saved choice against the currently attached inputs.
     * Matches by id first, then by type + address (ids change across reboots). Returns null
     * (= system default) when nothing is saved or the saved device has disappeared.
     */
    fun resolve(s: UserSettings, options: List<InputOption>): InputOption? {
        if (s.inputDeviceId == 0) return null
        return options.firstOrNull { it.id == s.inputDeviceId }
            ?: options.firstOrNull { it.type == s.inputDeviceType && it.address == s.inputDeviceAddress }
    }

    /** Finds the saved device among the attached inputs, or null for the default microphone. */
    fun find(context: Context, s: UserSettings): AudioDeviceInfo? {
        if (s.inputDeviceId == 0) return null
        val am = context.getSystemService(AudioManager::class.java) ?: return null
        val devices = am.getDevices(AudioManager.GET_DEVICES_INPUTS)
        val chosen = resolve(s, devices.map { InputOption(it.id, it.type, it.address ?: "", "") }) ?: return null
        return devices.firstOrNull { it.id == chosen.id }
    }

    fun label(d: AudioDeviceInfo): String {
        val type = typeName(d.type)
        val product = d.productName?.toString()?.trim().orEmpty()
        val address = d.address?.trim().orEmpty()
        val base = when {
            d.type == AudioDeviceInfo.TYPE_BUILTIN_MIC -> type
            product.isNotEmpty() && !product.equals(android.os.Build.MODEL, ignoreCase = true) -> "$type ($product)"
            else -> type
        }
        return if (d.type == AudioDeviceInfo.TYPE_BUILTIN_MIC && address.isNotEmpty()) "$base – $address" else base
    }

    fun typeName(type: Int): String = when (type) {
        AudioDeviceInfo.TYPE_BUILTIN_MIC -> "Built-in microphone"
        AudioDeviceInfo.TYPE_WIRED_HEADSET -> "Wired headset"
        AudioDeviceInfo.TYPE_USB_DEVICE -> "USB microphone"
        AudioDeviceInfo.TYPE_USB_HEADSET -> "USB headset"
        AudioDeviceInfo.TYPE_USB_ACCESSORY -> "USB accessory"
        AudioDeviceInfo.TYPE_BLUETOOTH_SCO -> "Bluetooth headset"
        AudioDeviceInfo.TYPE_BLUETOOTH_A2DP -> "Bluetooth audio"
        AudioDeviceInfo.TYPE_BLE_HEADSET -> "Bluetooth LE headset"
        AudioDeviceInfo.TYPE_HDMI -> "HDMI"
        AudioDeviceInfo.TYPE_LINE_ANALOG -> "Line in"
        AudioDeviceInfo.TYPE_LINE_DIGITAL -> "Digital line in"
        AudioDeviceInfo.TYPE_DOCK -> "Dock"
        AudioDeviceInfo.TYPE_IP -> "Network audio"
        AudioDeviceInfo.TYPE_BUS -> "Audio bus"
        else -> "Input device #$type"
    }
}
