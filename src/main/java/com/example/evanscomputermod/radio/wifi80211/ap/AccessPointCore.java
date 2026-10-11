package com.example.evanscomputermod.radio.wifi80211.ap;

import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.RxMeta;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.crypto.AesKeyWrap;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ccmp;
import com.example.evanscomputermod.radio.wifi80211.crypto.PnReplayWindow;
import com.example.evanscomputermod.radio.wifi80211.crypto.Ptk;
import com.example.evanscomputermod.radio.wifi80211.crypto.Wpa2Crypto;
import com.example.evanscomputermod.radio.wifi80211.frame.EapolKey;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;
import com.example.evanscomputermod.radio.wifi80211.frame.Frame80211;
import com.example.evanscomputermod.radio.wifi80211.frame.FrameControl;
import com.example.evanscomputermod.radio.wifi80211.frame.Frames;
import com.example.evanscomputermod.radio.wifi80211.frame.Ie;
import com.example.evanscomputermod.radio.wifi80211.frame.Kde;
import com.example.evanscomputermod.radio.wifi80211.frame.Llc;
import com.example.evanscomputermod.radio.wifi80211.frame.MacHeader;
import com.example.evanscomputermod.radio.wifi80211.frame.Mgmt;
import com.example.evanscomputermod.radio.wifi80211.frame.RsnIe;

import java.util.ArrayDeque;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.Deque;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.random.RandomGenerator;

/**
 * hostapd-like access point MAC/MLME and WPA2-PSK authenticator as a pure,
 * deterministic state machine. It never reads a clock or a global RNG: time
 * comes in through {@code nowMs} and randomness (ANonce, GTK) through the
 * {@link RandomGenerator} given at construction. Not thread-safe; drive it
 * from one thread.
 *
 * <p>Frames in and out are MPDUs without FCS. The AP is a layer-2 bridge:
 * uplink data is decrypted, replay-checked and handed to the cable side as
 * Ethernet with the client's source MAC; downlink Ethernet for associated
 * clients is encrypted with the client's TK (unicast) or the GTK (group).
 */
public final class AccessPointCore {

    private static final int RECENT_EVENTS = 32;

    private final ApConfig cfg;
    private final ApOutput out;
    private final RandomGenerator rng;
    private final MacAddress bssid;
    private final byte[] ssid;
    private final byte[] pmk;
    private final byte[] rsnIe;
    private final Map<MacAddress, Sta> stas = new LinkedHashMap<>();
    private final Deque<String> recentEvents = new ArrayDeque<>();

    private long now;
    private int seq;
    private long nextBeaconUs = -1;
    private long nextRekeyMs = -1;

    private byte[] gtk;
    private int gtkKeyId = 1;
    private long groupPn;
    private byte[] pendingGtk;
    private int pendingKeyId;

    private long beaconsSent;
    private long malformedFrames;

    private static final class Sta {
        final MacAddress mac;
        ClientStatus.State state = ClientStatus.State.AUTHENTICATED;
        ClientStatus.Handshake hs = ClientStatus.Handshake.NONE;
        int aid;
        byte[] rsnIe;
        byte[] anonce;
        Ptk ptk;
        boolean keysInstalled;
        long replayCounter;
        long stageFirstReplay;
        int attempts;
        long nextRetryMs;
        long txPn;
        final PnReplayWindow rxPn = new PnReplayWindow();
        int lastRssi;
        int lastRate;
        String lastError;
        long associatedAt = -1;
        long lastActivity;
        long rxFrames, txFrames, replayDrops, micFailures;
        boolean groupPending;

        Sta(MacAddress mac, long now) {
            this.mac = mac;
            this.lastActivity = now;
        }
    }

    public AccessPointCore(ApConfig cfg, ApOutput out, RandomGenerator rng) {
        this.cfg = cfg;
        this.out = out;
        this.rng = rng;
        this.bssid = cfg.bssid();
        this.ssid = cfg.ssid();
        if (cfg.security() == Security.WPA2_PSK) {
            this.pmk = Wpa2Crypto.pmk(cfg.passphrase(), ssid);
            this.rsnIe = RsnIe.wpa2PskCcmp().encode();
            this.gtk = random(16);
        } else {
            this.pmk = null;
            this.rsnIe = null;
        }
    }

    public ApConfig config() {
        return cfg;
    }

    // ------------------------------------------------------------------ inputs

    /** A frame (MPDU without FCS) received from the radio medium. */
    public void onReceive(byte[] frame80211, RxMeta meta, long nowMs) {
        now = nowMs;
        Frame80211 f;
        try {
            f = Frame80211.parse(frame80211);
        } catch (IllegalArgumentException e) {
            malformedFrames++;
            return;
        }
        if (meta == null) meta = RxMeta.NONE;
        if (meta.channel() > 0 && meta.channel() != cfg.channel()) return;
        MacHeader h = f.header();
        if (h.type() == FrameControl.TYPE_CTRL) return; // ACK/RTS/CTS/PS-Poll belong to the radio module
        MacAddress sa = h.addr2();
        if (sa.isGroup()) return;
        Sta sta = stas.get(sa);
        if (sta != null && (h.addr1().equals(bssid) || h.addr1().isGroup())) {
            sta.lastActivity = nowMs;
            sta.lastRssi = meta.rssiDbm();
            sta.lastRate = meta.rateKbps();
        }
        try {
            if (h.type() == FrameControl.TYPE_MGMT) {
                handleMgmt(h, f.body());
            } else if (h.type() == FrameControl.TYPE_DATA) {
                handleData(h, frame80211, f.body());
            }
        } catch (IllegalArgumentException e) {
            malformedFrames++;
            if (sta != null) sta.lastError = "malformed frame: " + e.getMessage();
        }
    }

    /** Advances timers: beacons, EAPOL retransmissions, inactivity, GTK rekey. */
    public void tick(long nowMs) {
        now = nowMs;
        long nowUs = nowMs * 1000;
        long intervalUs = cfg.beaconIntervalTu() * 1024L;
        if (nextBeaconUs < 0 || nowUs >= nextBeaconUs) {
            sendBeacon(nowUs);
            nextBeaconUs = nextBeaconUs < 0 ? nowUs + intervalUs : nextBeaconUs + intervalUs;
            while (nextBeaconUs <= nowUs) nextBeaconUs += intervalUs;
        }
        for (Sta sta : new ArrayList<>(stas.values())) {
            if (cfg.inactivityTimeoutMs() > 0 && nowMs - sta.lastActivity >= cfg.inactivityTimeoutMs()) {
                sta.lastError = "inactivity timeout";
                kick(sta, Mgmt.REASON_INACTIVITY, "inactive for " + (nowMs - sta.lastActivity) + " ms");
                continue;
            }
            if (sta.hs != ClientStatus.Handshake.NONE && sta.hs != ClientStatus.Handshake.DONE && nowMs >= sta.nextRetryMs) {
                retransmit(sta);
            }
        }
        if (cfg.security() == Security.WPA2_PSK && cfg.gtkRekeyIntervalMs() > 0) {
            if (nextRekeyMs < 0) nextRekeyMs = nowMs + cfg.gtkRekeyIntervalMs();
            else if (nowMs >= nextRekeyMs && pendingGtk == null) rekeyGtk();
        }
    }

    /** An Ethernet frame (no FCS) seen on the cable side of the bridge. */
    public void onWiredFrame(byte[] ethernetFrame) {
        EthernetFrame eth;
        try {
            eth = EthernetFrame.parse(ethernetFrame);
        } catch (IllegalArgumentException e) {
            malformedFrames++;
            return;
        }
        if (stas.containsKey(eth.src())) return; // our own client's frame echoed back: never loop it
        if (eth.dst().isGroup()) {
            sendGroupData(eth.dst(), eth.src(), eth.etherType(), eth.payload());
        } else {
            Sta sta = stas.get(eth.dst());
            if (sta != null && sta.state == ClientStatus.State.AUTHORIZED) {
                sendUnicastData(sta, eth.src(), eth.etherType(), eth.payload());
            }
        }
    }

    // ------------------------------------------------------------------ control

    /** Deauthenticates a station (e.g. kicked from the status page). Returns false if unknown. */
    public boolean deauthenticate(MacAddress mac, int reason) {
        Sta sta = stas.get(mac);
        if (sta == null) return false;
        kick(sta, reason, "deauthenticated by AP (reason " + reason + ")");
        return true;
    }

    /** Deauthenticates everyone with "leaving" (AP switched off or reconfigured). */
    public void shutdown() {
        out.transmitRadio(Frames.deauth(MacAddress.BROADCAST, bssid, bssid, nextSeq(), Mgmt.REASON_DEAUTH_LEAVING));
        for (Sta sta : new ArrayList<>(stas.values())) remove(sta, "AP shut down");
    }

    /** Starts a group key handshake now (normally driven by the rekey interval). */
    public void rekeyGtk() {
        if (cfg.security() != Security.WPA2_PSK || pendingGtk != null) return;
        pendingGtk = random(16);
        pendingKeyId = gtkKeyId == 1 ? 2 : 1;
        if (cfg.gtkRekeyIntervalMs() > 0) nextRekeyMs = now + cfg.gtkRekeyIntervalMs();
        event("GTK rekey started (key " + pendingKeyId + ")");
        for (Sta sta : stas.values()) {
            if (sta.state == ClientStatus.State.AUTHORIZED) startGroupHandshake(sta);
        }
        maybeFinishRekey();
    }

    // ------------------------------------------------------------------ status

    public List<ClientStatus> clients() {
        List<ClientStatus> list = new ArrayList<>(stas.size());
        for (Sta s : stas.values()) {
            list.add(new ClientStatus(s.mac, s.aid, s.state, s.hs, s.lastRssi, s.lastRate, s.lastError, s.associatedAt,
                    s.lastActivity, s.rxFrames, s.txFrames, s.replayDrops, s.micFailures));
        }
        return list;
    }

    public ClientStatus client(MacAddress mac) {
        for (ClientStatus c : clients()) if (c.mac().equals(mac)) return c;
        return null;
    }

    /** The most recent AP events (joins, leaves, errors), oldest first. */
    public List<String> recentEvents() {
        return List.copyOf(recentEvents);
    }

    public int currentGtkKeyId() {
        return gtkKeyId;
    }

    public boolean rekeyInProgress() {
        return pendingGtk != null;
    }

    public long beaconsSent() {
        return beaconsSent;
    }

    public long malformedFrames() {
        return malformedFrames;
    }

    // ------------------------------------------------------------------ management

    private void handleMgmt(MacHeader h, byte[] body) {
        int st = h.subtype();
        if (st == FrameControl.PROBE_REQ) {
            handleProbe(h, body);
            return;
        }
        if (!h.addr1().equals(bssid) || !h.addr3().equals(bssid)) return;
        switch (st) {
            case FrameControl.AUTH -> handleAuth(h, Mgmt.Auth.parse(body));
            case FrameControl.ASSOC_REQ -> handleAssoc(h, Mgmt.AssocRequest.parse(body, false), false);
            case FrameControl.REASSOC_REQ -> handleAssoc(h, Mgmt.AssocRequest.parse(body, true), true);
            case FrameControl.DEAUTH -> {
                Sta sta = stas.get(h.addr2());
                if (sta != null) remove(sta, "station deauthenticated (reason " + Mgmt.parseReason(body) + ")");
            }
            case FrameControl.DISASSOC -> {
                Sta sta = stas.get(h.addr2());
                if (sta != null) {
                    boolean wasAuthorized = sta.state == ClientStatus.State.AUTHORIZED;
                    resetAssociation(sta);
                    event(sta.mac + " disassociated (reason " + Mgmt.parseReason(body) + ")");
                    if (wasAuthorized) out.clientRemoved(sta.mac);
                    maybeFinishRekey();
                }
            }
            default -> { }
        }
    }

    private void handleProbe(MacHeader h, byte[] body) {
        if (!(h.addr1().isBroadcast() || h.addr1().equals(bssid))) return;
        if (!(h.addr3().isBroadcast() || h.addr3().equals(bssid))) return;
        List<Ie> ies = Ie.parseAll(body, 0, body.length);
        Ie ssidIe = Ie.find(ies, Ie.SSID);
        if (ssidIe == null) return;
        byte[] want = ssidIe.data();
        boolean wildcard = want.length == 0;
        if (wildcard ? cfg.hidden() : !Arrays.equals(want, ssid)) return;
        Ie ds = Ie.find(ies, Ie.DS_PARAMS);
        if (ds != null && ds.data().length == 1 && (ds.data()[0] & 0xFF) != cfg.channel()) return;
        Mgmt.Beacon resp = new Mgmt.Beacon(now * 1000, cfg.beaconIntervalTu(), capability(), bssIes(ssid, false));
        out.transmitRadio(Frames.mgmt(FrameControl.PROBE_RESP, h.addr2(), bssid, bssid, nextSeq(), resp.encode()));
    }

    private void handleAuth(MacHeader h, Mgmt.Auth a) {
        if (a.sequence() != 1) return;
        MacAddress sa = h.addr2();
        if (!cfg.macAllowed(sa)) {
            event(sa + " rejected by MAC filter");
            sendAuth(sa, a.algorithm(), Mgmt.STATUS_UNSPECIFIED);
            return;
        }
        if (a.algorithm() != Mgmt.AUTH_OPEN) {
            sendAuth(sa, a.algorithm(), Mgmt.STATUS_UNSUPPORTED_AUTH_ALG);
            return;
        }
        Sta sta = stas.get(sa);
        if (sta == null) {
            sta = new Sta(sa, now);
            stas.put(sa, sta);
        } else {
            boolean wasAuthorized = sta.state == ClientStatus.State.AUTHORIZED;
            resetAssociation(sta);
            if (wasAuthorized) out.clientRemoved(sa);
        }
        sendAuth(sa, Mgmt.AUTH_OPEN, Mgmt.STATUS_SUCCESS);
    }

    private void sendAuth(MacAddress to, int alg, int status) {
        byte[] body = new Mgmt.Auth(alg, 2, status, List.of()).encode();
        out.transmitRadio(Frames.mgmt(FrameControl.AUTH, to, bssid, bssid, nextSeq(), body));
    }

    private void handleAssoc(MacHeader h, Mgmt.AssocRequest req, boolean reassoc) {
        MacAddress sa = h.addr2();
        Sta sta = stas.get(sa);
        if (sta == null) {
            out.transmitRadio(Frames.deauth(sa, bssid, bssid, nextSeq(), Mgmt.REASON_CLASS2_FROM_NONAUTH));
            return;
        }
        int respSubtype = reassoc ? FrameControl.REASSOC_RESP : FrameControl.ASSOC_RESP;
        Ie ssidIe = Ie.find(req.ies(), Ie.SSID);
        if (ssidIe == null || !Arrays.equals(ssidIe.data(), ssid)) {
            sendAssocResp(sa, respSubtype, Mgmt.STATUS_UNSPECIFIED, 0);
            sta.lastError = "association with wrong SSID";
            return;
        }
        byte[] staRsn = null;
        if (cfg.security() == Security.WPA2_PSK) {
            Ie rsn = Ie.find(req.ies(), Ie.RSN);
            int status = Mgmt.STATUS_SUCCESS;
            if (rsn == null) {
                status = Mgmt.STATUS_INVALID_IE;
            } else {
                try {
                    RsnIe r = RsnIe.parse(rsn.data());
                    if (r.version() != 1) status = Mgmt.STATUS_UNSUPPORTED_RSN_VERSION;
                    else if (r.groupCipher() != RsnIe.SUITE_CCMP) status = Mgmt.STATUS_INVALID_GROUP_CIPHER;
                    else if (r.pairwiseCiphers().size() != 1 || r.pairwiseCiphers().get(0) != RsnIe.SUITE_CCMP) status = Mgmt.STATUS_INVALID_PAIRWISE_CIPHER;
                    else if (r.akms().size() != 1 || r.akms().get(0) != RsnIe.AKM_PSK) status = Mgmt.STATUS_INVALID_AKMP;
                } catch (IllegalArgumentException e) {
                    status = Mgmt.STATUS_INVALID_IE;
                }
            }
            if (status != Mgmt.STATUS_SUCCESS) {
                sta.lastError = "association rejected: RSN element (status " + status + ")";
                sendAssocResp(sa, respSubtype, status, 0);
                return;
            }
            staRsn = rsn.encode();
        }
        boolean wasAuthorized = sta.state == ClientStatus.State.AUTHORIZED;
        resetAssociation(sta);
        if (wasAuthorized) out.clientRemoved(sa);
        int aid = freeAid();
        if (aid == 0) {
            sta.lastError = "AP full";
            sendAssocResp(sa, respSubtype, Mgmt.STATUS_AP_FULL, 0);
            return;
        }
        sta.aid = aid;
        sta.rsnIe = staRsn;
        sta.state = ClientStatus.State.ASSOCIATED;
        sta.associatedAt = now;
        sta.lastError = null;
        sendAssocResp(sa, respSubtype, Mgmt.STATUS_SUCCESS, aid);
        event(sa + " associated (AID " + aid + ")");
        if (cfg.security() == Security.WPA2_PSK) {
            start4Way(sta);
        } else {
            authorize(sta);
        }
    }

    private void sendAssocResp(MacAddress to, int subtype, int status, int aid) {
        byte[] body = new Mgmt.AssocResponse(capability(), status, aid, Ie.rates(cfg.channel())).encode();
        out.transmitRadio(Frames.mgmt(subtype, to, bssid, bssid, nextSeq(), body));
    }

    private int freeAid() {
        int count = 0;
        boolean[] used = new boolean[ApConfig.MAX_AID + 1];
        for (Sta s : stas.values()) {
            if (s.aid > 0) {
                used[s.aid] = true;
                count++;
            }
        }
        if (count >= cfg.maxClients()) return 0;
        for (int i = 1; i <= ApConfig.MAX_AID; i++) if (!used[i]) return i;
        return 0;
    }

    private void sendBeacon(long nowUs) {
        byte[] beaconSsid = cfg.hidden() ? new byte[0] : ssid;
        Mgmt.Beacon b = new Mgmt.Beacon(nowUs, cfg.beaconIntervalTu(), capability(), bssIes(beaconSsid, true));
        out.transmitRadio(Frames.mgmt(FrameControl.BEACON, MacAddress.BROADCAST, bssid, bssid, nextSeq(), b.encode()));
        beaconsSent++;
    }

    private int capability() {
        int cap = Mgmt.CAP_ESS | Mgmt.CAP_SHORT_SLOT;
        if (cfg.security() == Security.WPA2_PSK) cap |= Mgmt.CAP_PRIVACY;
        return cap;
    }

    /** Elements in the order 802.11 requires: SSID, Rates, DS Params, TIM (beacon only), RSN, Extended Rates. */
    private List<Ie> bssIes(byte[] ssidField, boolean beacon) {
        List<Ie> ies = new ArrayList<>();
        ies.add(Ie.ssid(ssidField));
        List<Ie> rates = Ie.rates(cfg.channel());
        ies.add(rates.get(0));
        ies.add(Ie.dsParams(cfg.channel()));
        if (beacon) ies.add(Ie.tim());
        if (rsnIe != null) ies.add(RsnIe.wpa2PskCcmp().toIe());
        if (rates.size() > 1) ies.add(rates.get(1));
        return ies;
    }

    // ------------------------------------------------------------------ 4-way and group handshakes

    private void start4Way(Sta sta) {
        sta.anonce = random(Wpa2Crypto.NONCE_LEN);
        sta.ptk = null;
        sta.keysInstalled = false;
        sta.hs = ClientStatus.Handshake.PTK_START;
        sta.attempts = 0;
        sta.txPn = 0;
        sta.rxPn.reset(0);
        sendM1(sta);
    }

    private void sendM1(Sta sta) {
        beginAttempt(sta);
        EapolKey m1 = EapolKey.of(EapolKey.KI_M1, 16, sta.replayCounter, sta.anonce, null, null);
        sendEapol(sta, m1.encode());
    }

    private void sendM3(Sta sta) {
        beginAttempt(sta);
        byte[] keyData = Kde.pad(Kde.concat(rsnIe, Kde.gtk(gtkKeyId, false, gtk)));
        byte[] wrapped = AesKeyWrap.wrap(sta.ptk.kek(), keyData);
        EapolKey m3 = EapolKey.of(EapolKey.KI_M3, 16, sta.replayCounter, sta.anonce, EapolKey.rscFromPn(groupPn), wrapped);
        sendEapol(sta, withMic(m3, sta.ptk));
    }

    private void startGroupHandshake(Sta sta) {
        sta.groupPending = true;
        sta.hs = ClientStatus.Handshake.GTK_REKEYING;
        sta.attempts = 0;
        sendGroupM1(sta);
    }

    private void sendGroupM1(Sta sta) {
        beginAttempt(sta);
        byte[] keyData = Kde.pad(Kde.gtk(pendingKeyId, false, pendingGtk));
        byte[] wrapped = AesKeyWrap.wrap(sta.ptk.kek(), keyData);
        EapolKey g1 = EapolKey.of(EapolKey.KI_G1, 0, sta.replayCounter, random(32), EapolKey.rscFromPn(0), wrapped);
        sendEapol(sta, withMic(g1, sta.ptk));
    }

    /** Each transmission uses a fresh replay counter; responses may echo any counter of the current stage. */
    private void beginAttempt(Sta sta) {
        sta.replayCounter++;
        if (sta.attempts == 0) sta.stageFirstReplay = sta.replayCounter;
        sta.attempts++;
        sta.nextRetryMs = now + cfg.eapolTimeoutMs();
    }

    private static byte[] withMic(EapolKey k, Ptk ptk) {
        return k.withMic(Wpa2Crypto.eapolMic(ptk.kck(), k.encodeForMic())).encode();
    }

    private void retransmit(Sta sta) {
        if (sta.attempts >= cfg.eapolMaxAttempts()) {
            boolean group = sta.hs == ClientStatus.Handshake.GTK_REKEYING;
            String what = group ? "group key handshake timeout" : "4-way handshake timeout";
            sta.lastError = sta.lastError == null ? what : sta.lastError + "; " + what;
            kick(sta, group ? Mgmt.REASON_GROUP_KEY_TIMEOUT : Mgmt.REASON_4WAY_TIMEOUT, what);
            return;
        }
        switch (sta.hs) {
            case PTK_START -> sendM1(sta);
            case PTK_NEGOTIATING -> sendM3(sta);
            case GTK_REKEYING -> sendGroupM1(sta);
            default -> { }
        }
    }

    private void handleEapol(Sta sta, byte[] pdu) {
        if (cfg.security() != Security.WPA2_PSK) return;
        EapolKey k = EapolKey.parse(pdu);
        if (k == null || k.descriptorType() != EapolKey.DESC_RSN) return;
        if (k.descriptorVersion() != EapolKey.KI_VERSION_HMAC_SHA1_AES) {
            sta.lastError = "unsupported EAPOL-Key descriptor version " + k.descriptorVersion();
            return;
        }
        if (!k.has(EapolKey.KI_MIC) || k.has(EapolKey.KI_ACK)) return;
        if (k.has(EapolKey.KI_REQUEST)) {
            if (k.has(EapolKey.KI_ERROR) && sta.ptk != null && micValid(k, sta.ptk)) {
                sta.micFailures++;
                sta.lastError = "station reported a CCMP MIC failure";
            }
            return;
        }
        boolean inStage = k.replayCounter() >= sta.stageFirstReplay && k.replayCounter() <= sta.replayCounter;
        if (k.has(EapolKey.KI_PAIRWISE)) {
            if (sta.hs == ClientStatus.Handshake.PTK_START && !k.has(EapolKey.KI_SECURE)) {
                if (!inStage) return;
                Ptk ptk = Wpa2Crypto.derivePtk(pmk, bssid, sta.mac, sta.anonce, k.nonce());
                if (!micValid(k, ptk)) {
                    sta.micFailures++;
                    sta.lastError = "4-way M2 MIC mismatch (wrong passphrase?)";
                    event(sta.mac + ": " + sta.lastError);
                    return;
                }
                byte[] echoed = Kde.findElement(k.keyData(), Ie.RSN);
                if (echoed == null || !Arrays.equals(echoed, sta.rsnIe)) {
                    sta.lastError = "RSN element in M2 differs from association request";
                    kick(sta, Mgmt.REASON_IE_DIFFERENT, sta.lastError);
                    return;
                }
                sta.ptk = ptk;
                sta.hs = ClientStatus.Handshake.PTK_NEGOTIATING;
                sta.attempts = 0;
                sendM3(sta);
            } else if (sta.hs == ClientStatus.Handshake.PTK_NEGOTIATING && k.has(EapolKey.KI_SECURE)) {
                if (!inStage) return;
                if (!micValid(k, sta.ptk)) {
                    sta.micFailures++;
                    sta.lastError = "4-way M4 MIC mismatch";
                    return;
                }
                sta.keysInstalled = true;
                sta.hs = ClientStatus.Handshake.DONE;
                sta.lastError = null;
                authorize(sta);
                if (pendingGtk != null) startGroupHandshake(sta);
            }
        } else if (sta.hs == ClientStatus.Handshake.GTK_REKEYING && k.has(EapolKey.KI_SECURE)) {
            if (!inStage || !micValid(k, sta.ptk)) {
                if (inStage) {
                    sta.micFailures++;
                    sta.lastError = "group key message 2 MIC mismatch";
                }
                return;
            }
            sta.groupPending = false;
            sta.hs = ClientStatus.Handshake.DONE;
            maybeFinishRekey();
        }
    }

    private static boolean micValid(EapolKey k, Ptk ptk) {
        return Wpa2Crypto.micEquals(Wpa2Crypto.eapolMic(ptk.kck(), k.encodeForMic()), k.mic());
    }

    private void maybeFinishRekey() {
        if (pendingGtk == null) return;
        for (Sta s : stas.values()) if (s.groupPending) return;
        gtk = pendingGtk;
        gtkKeyId = pendingKeyId;
        groupPn = 0;
        pendingGtk = null;
        event("GTK rekey complete (key " + gtkKeyId + ")");
    }

    private void authorize(Sta sta) {
        sta.state = ClientStatus.State.AUTHORIZED;
        event(sta.mac + " authorized");
        out.clientAuthorized(sta.mac);
    }

    // ------------------------------------------------------------------ data path

    private void handleData(MacHeader h, byte[] raw, byte[] body) {
        if (!h.addr1().equals(bssid)) return;
        MacAddress sa = h.addr2();
        Sta sta = stas.get(sa);
        if (!h.toDs() || h.fromDs()) return; // WDS / IBSS frames are not ours
        if (sta == null || sta.state == ClientStatus.State.AUTHENTICATED) {
            out.transmitRadio(Frames.deauth(sa, bssid, bssid, nextSeq(), Mgmt.REASON_CLASS3_FROM_NONASSOC));
            return;
        }
        boolean wpa2 = cfg.security() == Security.WPA2_PSK;
        boolean decrypted = false;
        if (h.isProtected()) {
            if (!wpa2 || sta.ptk == null) return;
            Ccmp.Decrypted d = Ccmp.decrypt(sta.ptk.tk(), raw);
            if (d == null || d.keyId() != 0) {
                sta.lastError = "CCMP decryption failed";
                return;
            }
            int idx = h.isQos() ? h.tid() : PnReplayWindow.NON_QOS;
            if (!sta.rxPn.accept(idx, d.pn())) {
                sta.replayDrops++;
                sta.lastError = "replayed PN " + d.pn() + " dropped";
                return;
            }
            body = Arrays.copyOfRange(d.mpdu(), h.length(), d.mpdu().length);
            decrypted = true;
        }
        if (FrameControl.isNullData(h.frameControl())) return; // keep-alive / power save only
        Llc.Decap llc = Llc.decap(body);
        if (llc.etherType() == EthernetFrame.ETHERTYPE_EAPOL) {
            handleEapol(sta, llc.payload());
            return;
        }
        if (sta.state != ClientStatus.State.AUTHORIZED) return;
        if (wpa2 && !decrypted) return; // plaintext data on a protected BSS
        sta.rxFrames++;
        MacAddress da = h.addr3();
        if (da.isGroup()) {
            out.transmitWired(new EthernetFrame(da, sa, llc.etherType(), llc.payload()).encode(), sa);
            if (!cfg.clientIsolation()) sendGroupData(da, sa, llc.etherType(), llc.payload());
            return;
        }
        Sta target = stas.get(da);
        if (target != null) {
            if (!cfg.clientIsolation() && target.state == ClientStatus.State.AUTHORIZED) {
                sendUnicastData(target, sa, llc.etherType(), llc.payload());
            }
            return;
        }
        out.transmitWired(new EthernetFrame(da, sa, llc.etherType(), llc.payload()).encode(), sa);
    }

    private void sendUnicastData(Sta sta, MacAddress sa, int etherType, byte[] payload) {
        byte[] mpdu = Frames.data(FrameControl.FROM_DS, sta.mac, bssid, sa, nextSeq(), etherType, payload);
        if (cfg.security() == Security.WPA2_PSK) mpdu = Ccmp.encrypt(sta.ptk.tk(), mpdu, ++sta.txPn, 0);
        sta.txFrames++;
        out.transmitRadio(mpdu);
    }

    private void sendGroupData(MacAddress da, MacAddress sa, int etherType, byte[] payload) {
        boolean anyone = false;
        for (Sta s : stas.values()) {
            if (s.state == ClientStatus.State.AUTHORIZED) {
                anyone = true;
                s.txFrames++;
            }
        }
        if (!anyone) return;
        byte[] mpdu = Frames.data(FrameControl.FROM_DS, da, bssid, sa, nextSeq(), etherType, payload);
        if (cfg.security() == Security.WPA2_PSK) mpdu = Ccmp.encrypt(gtk, mpdu, ++groupPn, gtkKeyId);
        out.transmitRadio(mpdu);
    }

    private void sendEapol(Sta sta, byte[] pdu) {
        byte[] mpdu = Frames.data(FrameControl.FROM_DS, sta.mac, bssid, bssid, nextSeq(), EthernetFrame.ETHERTYPE_EAPOL, pdu);
        if (sta.keysInstalled) mpdu = Ccmp.encrypt(sta.ptk.tk(), mpdu, ++sta.txPn, 0);
        out.transmitRadio(mpdu);
    }

    // ------------------------------------------------------------------ bookkeeping

    /** Back to "authenticated, not associated": AID freed, keys and handshake dropped. */
    private void resetAssociation(Sta sta) {
        sta.state = ClientStatus.State.AUTHENTICATED;
        sta.hs = ClientStatus.Handshake.NONE;
        sta.aid = 0;
        sta.ptk = null;
        sta.keysInstalled = false;
        sta.anonce = null;
        sta.attempts = 0;
        sta.groupPending = false;
        sta.associatedAt = -1;
    }

    private void kick(Sta sta, int reason, String why) {
        out.transmitRadio(Frames.deauth(sta.mac, bssid, bssid, nextSeq(), reason));
        remove(sta, why);
    }

    private void remove(Sta sta, String why) {
        stas.remove(sta.mac);
        event(sta.mac + " removed: " + why);
        if (sta.state == ClientStatus.State.AUTHORIZED) out.clientRemoved(sta.mac);
        maybeFinishRekey();
    }

    private void event(String s) {
        if (recentEvents.size() == RECENT_EVENTS) recentEvents.removeFirst();
        recentEvents.addLast(now + " ms: " + s);
    }

    private int nextSeq() {
        int s = seq;
        seq = (seq + 1) & 0xFFF;
        return s;
    }

    private byte[] random(int n) {
        byte[] b = new byte[n];
        rng.nextBytes(b);
        return b;
    }
}
