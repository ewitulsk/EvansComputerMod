package com.example.evanscomputermod.radio.wifi80211.sta;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.frame.RsnIe;

import java.nio.charset.StandardCharsets;

/**
 * One BSS seen in beacons or probe responses.
 *
 * @param ssid  SSID bytes; empty for a hidden network seen only in beacons
 * @param rsn   the advertised RSN element, or null for an open network
 * @param rsnIe the RSN element bytes exactly as advertised (compared against M3), or null
 */
public record ScanResult(MacAddress bssid, byte[] ssid, int channel, int rssiDbm, int capability, RsnIe rsn,
                         byte[] rsnIe, long lastSeenMs) {

    public String ssidString() {
        return new String(ssid, StandardCharsets.UTF_8);
    }

    public boolean hidden() {
        return ssid.length == 0;
    }
}
