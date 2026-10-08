package com.example.evanscomputermod.radio.wifi.ap;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.crypto.Wpa2Crypto;

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.LinkedHashSet;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * What a player can set on an Access Point, minus the passphrase (which lives
 * only in the block entity's server-side NBT and never travels to clients).
 * Pure: validated here, turned into an {@link ApConfig} for the core.
 *
 * @param channelSetting 0 = auto (least busy of 1/6/11), else an entry of {@link #CHANNEL_CHOICES}
 */
public record ApSettings(String ssid, boolean hidden, Security security, int channelSetting, int txPowerDbm,
                         boolean clientIsolation, ApConfig.MacFilterMode filterMode, List<MacAddress> filterMacs) {

    /** Channel choices offered by the GUI (0 = auto). 5 GHz: UNII-1 and UNII-3, 20 MHz. */
    public static final int[] CHANNEL_CHOICES = {0, 1, 6, 11, 36, 40, 44, 48, 149, 153, 157, 161, 165};
    /** Auto picks among the non-overlapping 2.4 GHz channels, in this order of preference on ties. */
    public static final int[] AUTO_CHANNELS = {6, 1, 11};
    public static final int MAX_TX_POWER_DBM = 20;
    public static final int MIN_TX_POWER_DBM = 0;
    public static final int MAX_FILTER_MACS = 32;

    public ApSettings {
        ssid = ssid == null ? "" : ssid;
        security = security == null ? Security.OPEN : security;
        filterMode = filterMode == null ? ApConfig.MacFilterMode.OFF : filterMode;
        filterMacs = filterMacs == null ? List.of() : List.copyOf(filterMacs);
    }

    /** Factory settings: open network named after the BSSID's last two octets, auto channel, full power. */
    public static ApSettings defaults(MacAddress bssid) {
        String tail = bssid.toString().substring(12).replace(":", "").toUpperCase(Locale.ROOT);
        return new ApSettings("ECM-" + tail, false, Security.OPEN, 0, MAX_TX_POWER_DBM, false,
                ApConfig.MacFilterMode.OFF, List.of());
    }

    public static boolean validChannelSetting(int ch) {
        for (int c : CHANNEL_CHOICES) if (c == ch) return true;
        return false;
    }

    /**
     * Problem with these settings and the passphrase that would be in force
     * (the new one, or the stored one when the field was left empty), or null if fine.
     */
    public String validate(String effectivePassphrase) {
        int len = ssid.getBytes(StandardCharsets.UTF_8).length;
        if (len < 1 || len > 32) return "SSID must be 1-32 bytes";
        for (int i = 0; i < ssid.length(); i++) {
            if (Character.isISOControl(ssid.charAt(i))) return "SSID has control characters";
        }
        if (!validChannelSetting(channelSetting)) return "channel " + channelSetting + " is not offered";
        if (txPowerDbm < MIN_TX_POWER_DBM || txPowerDbm > MAX_TX_POWER_DBM)
            return "transmit power must be " + MIN_TX_POWER_DBM + "-" + MAX_TX_POWER_DBM + " dBm";
        if (filterMacs.size() > MAX_FILTER_MACS) return "at most " + MAX_FILTER_MACS + " MAC filter entries";
        for (MacAddress m : filterMacs) if (m.isGroup()) return "filter entry " + m + " is a group address";
        if (security == Security.WPA2_PSK) {
            if (effectivePassphrase == null || effectivePassphrase.isEmpty()) return "WPA2 needs a passphrase";
            if (!Wpa2Crypto.isValidPassphrase(effectivePassphrase))
                return "passphrase must be 8-63 printable ASCII characters or 64 hex digits";
        }
        return null;
    }

    /** The core's configuration on {@code channel} (already resolved from auto). */
    public ApConfig toConfig(MacAddress bssid, int channel, String passphrase) {
        ApConfig.Builder b = ApConfig.builder(bssid, ssid).hidden(hidden).channel(channel).clientIsolation(clientIsolation)
                .macFilter(filterMode, new LinkedHashSet<>(filterMacs));
        if (security == Security.WPA2_PSK) b.wpa2(passphrase);
        else b.open();
        return b.build();
    }

    /**
     * Auto channel: the least busy of {@link #AUTO_CHANNELS} given the in-channel
     * power heard on each (dBm, -infinity = silent); ties keep the preference order.
     */
    public static int pickAutoChannel(Map<Integer, Double> powerDbm) {
        int best = AUTO_CHANNELS[0];
        double bestP = Double.POSITIVE_INFINITY;
        for (int c : AUTO_CHANNELS) {
            double p = powerDbm.getOrDefault(c, Double.NEGATIVE_INFINITY);
            if (p < bestP - 1e-9) {
                best = c;
                bestP = p;
            }
        }
        return best;
    }

    /** Parses a MAC list separated by commas, spaces or newlines. Throws with a readable message. */
    public static List<MacAddress> parseMacList(String text) {
        List<MacAddress> out = new ArrayList<>();
        if (text == null) return out;
        for (String part : text.split("[,;\\s]+")) {
            if (part.isBlank()) continue;
            try {
                MacAddress m = MacAddress.parse(part);
                if (!out.contains(m)) out.add(m);
            } catch (IllegalArgumentException e) {
                throw new IllegalArgumentException("not a MAC address: " + part);
            }
        }
        return out;
    }

    public static String formatMacList(List<MacAddress> macs) {
        StringBuilder sb = new StringBuilder();
        for (MacAddress m : macs) {
            if (sb.length() > 0) sb.append(", ");
            sb.append(m);
        }
        return sb.toString();
    }
}
