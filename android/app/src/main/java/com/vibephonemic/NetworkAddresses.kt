package com.vibephonemic

import java.net.Inet4Address
import java.net.NetworkInterface

data class LocalAddress(val label: String, val ip: String)

object NetworkAddresses {
    /** Non-loopback IPv4 addresses of interfaces that are up, Wi-Fi first. */
    fun list(): List<LocalAddress> = try {
        NetworkInterface.getNetworkInterfaces().toList()
            .filter { it.isUp && !it.isLoopback }
            .flatMap { nif ->
                nif.inetAddresses.toList()
                    .filterIsInstance<Inet4Address>()
                    .filter { !it.isLoopbackAddress && !it.isLinkLocalAddress }
                    .map { LocalAddress(labelFor(nif.name), it.hostAddress ?: "") }
            }
            .filter { it.ip.isNotEmpty() }
            .sortedBy { order(it.label) }
    } catch (_: Exception) {
        emptyList()
    }

    fun labelFor(name: String): String = when {
        name.startsWith("wlan") -> "Wi-Fi"
        name.startsWith("swlan") || name.startsWith("ap") || name.startsWith("softap") -> "Hotspot"
        name.startsWith("rndis") || name.startsWith("usb") || name.startsWith("ncm") -> "USB tethering"
        name.startsWith("eth") -> "Ethernet"
        name.startsWith("rmnet") || name.startsWith("ccmni") || name.startsWith("radio") -> "Mobile data"
        name.startsWith("bt-pan") || name.startsWith("bnep") -> "Bluetooth tethering"
        name.startsWith("tun") || name.startsWith("ppp") -> "VPN"
        else -> name
    }

    private fun order(label: String) = when (label) {
        "Wi-Fi" -> 0; "USB tethering" -> 1; "Hotspot" -> 2; "Ethernet" -> 3
        "Mobile data" -> 9; else -> 5
    }
}
