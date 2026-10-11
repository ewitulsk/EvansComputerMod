package com.example.evanscomputermod.radio.wifi80211.ap;

import com.example.evanscomputermod.radio.wifi80211.Channels;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.crypto.Wpa2Crypto;

import java.nio.charset.StandardCharsets;
import java.util.Set;

/**
 * Access point configuration. Build with {@link #builder(MacAddress, String)};
 * the constructor validates everything so a core never runs with a bad config.
 *
 * @param bssid                 the AP's MAC address (BSSID)
 * @param ssid                  1..32 bytes
 * @param hidden                beacons carry an empty SSID and wildcard probes go unanswered
 * @param passphrase            WPA2 passphrase (8..63 ASCII or 64 hex); ignored for OPEN
 * @param beaconIntervalTu      beacon interval in TU (1 TU = 1024 us); 100 = 102.4 ms
 * @param macFilterMode         OFF, ALLOW (only listed MACs) or DENY (all but listed)
 * @param inactivityTimeoutMs   0 disables
 * @param gtkRekeyIntervalMs    0 disables
 * @param eapolTimeoutMs        EAPOL-Key retransmission timeout
 * @param eapolMaxAttempts      transmissions of one handshake message before giving up
 */
public record ApConfig(MacAddress bssid, byte[] ssid, boolean hidden, Security security, String passphrase,
                       int channel, int beaconIntervalTu, boolean clientIsolation, MacFilterMode macFilterMode,
                       Set<MacAddress> macFilter, long inactivityTimeoutMs, long gtkRekeyIntervalMs,
                       long eapolTimeoutMs, int eapolMaxAttempts, int maxClients) {

    public enum MacFilterMode { OFF, ALLOW, DENY }

    public static final int MAX_AID = 2007;

    public ApConfig {
        if (bssid == null || bssid.isGroup()) throw new IllegalArgumentException("BSSID must be an individual address");
        if (ssid == null || ssid.length < 1 || ssid.length > 32) throw new IllegalArgumentException("SSID must be 1..32 bytes");
        ssid = ssid.clone();
        if (security == null) throw new IllegalArgumentException("security required");
        if (security == Security.WPA2_PSK && !Wpa2Crypto.isValidPassphrase(passphrase)) {
            throw new IllegalArgumentException("WPA2 passphrase must be 8..63 printable ASCII characters or 64 hex digits");
        }
        if (!Channels.isValid(channel)) throw new IllegalArgumentException("invalid channel " + channel);
        if (beaconIntervalTu < 1 || beaconIntervalTu > 65535) throw new IllegalArgumentException("beacon interval out of range");
        macFilterMode = macFilterMode == null ? MacFilterMode.OFF : macFilterMode;
        macFilter = macFilter == null ? Set.of() : Set.copyOf(macFilter);
        if (inactivityTimeoutMs < 0 || gtkRekeyIntervalMs < 0) throw new IllegalArgumentException("negative timeout");
        if (eapolTimeoutMs < 1 || eapolMaxAttempts < 1) throw new IllegalArgumentException("EAPOL retry settings must be positive");
        if (maxClients < 1 || maxClients > MAX_AID) throw new IllegalArgumentException("maxClients must be 1.." + MAX_AID);
    }

    @Override
    public byte[] ssid() {
        return ssid.clone();
    }

    public String ssidString() {
        return new String(ssid, StandardCharsets.UTF_8);
    }

    public boolean macAllowed(MacAddress mac) {
        return switch (macFilterMode) {
            case OFF -> true;
            case ALLOW -> macFilter.contains(mac);
            case DENY -> !macFilter.contains(mac);
        };
    }

    @Override
    public String toString() {
        return "ApConfig[bssid=" + bssid + ", ssid=" + ssidString() + ", hidden=" + hidden + ", security=" + security
                + ", channel=" + channel + ", isolation=" + clientIsolation + ", filter=" + macFilterMode + "]";
    }

    public static Builder builder(MacAddress bssid, String ssid) {
        return new Builder(bssid, ssid.getBytes(StandardCharsets.UTF_8));
    }

    public Builder toBuilder() {
        Builder b = new Builder(bssid, ssid);
        b.hidden = hidden;
        b.security = security;
        b.passphrase = passphrase;
        b.channel = channel;
        b.beaconIntervalTu = beaconIntervalTu;
        b.clientIsolation = clientIsolation;
        b.macFilterMode = macFilterMode;
        b.macFilter = macFilter;
        b.inactivityTimeoutMs = inactivityTimeoutMs;
        b.gtkRekeyIntervalMs = gtkRekeyIntervalMs;
        b.eapolTimeoutMs = eapolTimeoutMs;
        b.eapolMaxAttempts = eapolMaxAttempts;
        b.maxClients = maxClients;
        return b;
    }

    /** Defaults follow hostapd: open, channel 6, 100 TU, 5 min inactivity, 1 h GTK rekey, 1 s x 4 EAPOL retries. */
    public static final class Builder {
        private final MacAddress bssid;
        private final byte[] ssid;
        private boolean hidden;
        private Security security = Security.OPEN;
        private String passphrase;
        private int channel = 6;
        private int beaconIntervalTu = 100;
        private boolean clientIsolation;
        private MacFilterMode macFilterMode = MacFilterMode.OFF;
        private Set<MacAddress> macFilter = Set.of();
        private long inactivityTimeoutMs = 300_000;
        private long gtkRekeyIntervalMs = 3_600_000;
        private long eapolTimeoutMs = 1_000;
        private int eapolMaxAttempts = 4;
        private int maxClients = MAX_AID;

        private Builder(MacAddress bssid, byte[] ssid) {
            this.bssid = bssid;
            this.ssid = ssid;
        }

        public Builder hidden(boolean v) { hidden = v; return this; }
        public Builder open() { security = Security.OPEN; passphrase = null; return this; }
        public Builder wpa2(String pass) { security = Security.WPA2_PSK; passphrase = pass; return this; }
        public Builder channel(int v) { channel = v; return this; }
        public Builder beaconIntervalTu(int v) { beaconIntervalTu = v; return this; }
        public Builder clientIsolation(boolean v) { clientIsolation = v; return this; }
        public Builder macFilter(MacFilterMode mode, Set<MacAddress> macs) { macFilterMode = mode; macFilter = macs; return this; }
        public Builder inactivityTimeoutMs(long v) { inactivityTimeoutMs = v; return this; }
        public Builder gtkRekeyIntervalMs(long v) { gtkRekeyIntervalMs = v; return this; }
        public Builder eapolTimeoutMs(long v) { eapolTimeoutMs = v; return this; }
        public Builder eapolMaxAttempts(int v) { eapolMaxAttempts = v; return this; }
        public Builder maxClients(int v) { maxClients = v; return this; }

        public ApConfig build() {
            return new ApConfig(bssid, ssid, hidden, security, passphrase, channel, beaconIntervalTu, clientIsolation,
                    macFilterMode, macFilter, inactivityTimeoutMs, gtkRekeyIntervalMs, eapolTimeoutMs, eapolMaxAttempts, maxClients);
        }
    }
}
