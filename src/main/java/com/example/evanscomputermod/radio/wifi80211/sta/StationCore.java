package com.example.evanscomputermod.radio.wifi80211.sta;

import com.example.evanscomputermod.radio.wifi80211.Channels;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.RxMeta;
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

import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.Arrays;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.random.RandomGenerator;

/**
 * Station (client) side of 802.11 + WPA2-PSK as a pure state machine: active
 * scan, Open System authentication, association, the supplicant side of the
 * 4-way and group key handshakes, and CCMP data. It mirrors what the Rust
 * {@code ecm-wifi} crate and {@code wpa_supplicant} do, so the Java AP can be
 * tested end to end without a kernel. Frames are MPDUs without FCS.
 */
public final class StationCore {

    public enum State { IDLE, AUTHENTICATING, ASSOCIATING, HANDSHAKE, CONNECTED }

    public static final long MLME_TIMEOUT_MS = 200;
    public static final int MLME_ATTEMPTS = 3;
    public static final long HANDSHAKE_TIMEOUT_MS = 10_000;
    /**
     * Beacon loss: with nothing at all heard from the AP for this long (about 20
     * beacon intervals) a joined station gives the link up, forgets the BSS and
     * goes back to scanning, as Linux mac80211 does. Without it a station that
     * flew out of range would believe it is connected forever.
     */
    public static final long BEACON_LOSS_MS = 2_000;

    private final MacAddress mac;
    private final StaOutput out;
    private final RandomGenerator rng;
    private final Map<MacAddress, ScanResult> scan = new LinkedHashMap<>();

    private State state = State.IDLE;
    private int seq;
    private long now;
    private String lastError;
    private int lastReason;

    // target BSS
    private MacAddress bssid;
    private byte[] ssid;
    private int channel;
    private boolean rsn;
    private byte[] apRsnIe;
    private byte[] ownRsnIe;
    private byte[] pmk;
    private int aid;
    private int attempts;
    private long deadline;
    private long lastHeardMs;

    // keys
    private long lastReplay = -1;
    private byte[] anonce;
    private byte[] snonce;
    private Ptk tptk;
    private Ptk ptk;
    private long txPn;
    private final PnReplayWindow rxPn = new PnReplayWindow();
    private final byte[][] gtks = new byte[4][];
    private final PnReplayWindow[] rxGroupPn = new PnReplayWindow[4];

    private long rxFrames, txFrames, replayDrops, decryptFailures, m4Sent;
    private boolean plaintextEapol;
    private boolean qosData;

    public StationCore(MacAddress mac, StaOutput out, RandomGenerator rng) {
        if (mac.isGroup()) throw new IllegalArgumentException("station MAC must be an individual address");
        this.mac = mac;
        this.out = out;
        this.rng = rng;
    }

    // ------------------------------------------------------------------ API

    /** Sends a probe request: wildcard if {@code ssidOrNull} is null, otherwise directed (finds hidden networks). */
    public void probe(String ssidOrNull, int channel, long nowMs) {
        now = nowMs;
        byte[] want = ssidOrNull == null ? new byte[0] : ssidOrNull.getBytes(StandardCharsets.UTF_8);
        List<Ie> ies = new ArrayList<>();
        ies.add(Ie.ssid(want));
        ies.addAll(Ie.rates(channel));
        ies.add(Ie.dsParams(channel));
        out.transmitRadio(Frames.mgmt(FrameControl.PROBE_REQ, MacAddress.BROADCAST, mac, MacAddress.BROADCAST, nextSeq(),
                Ie.encodeAll(ies)));
    }

    public List<ScanResult> scanResults() {
        return List.copyOf(scan.values());
    }

    /**
     * Starts joining the strongest BSS advertising {@code ssid}. {@code passphrase}
     * null means an open network. Returns false (with {@link #lastError()}) if no
     * matching BSS was seen or the security does not match.
     */
    public boolean connect(String ssidName, String passphrase, long nowMs) {
        now = nowMs;
        byte[] want = ssidName.getBytes(StandardCharsets.UTF_8);
        ScanResult best = null;
        for (ScanResult r : scan.values()) {
            if (Arrays.equals(r.ssid(), want) && (best == null || r.rssiDbm() > best.rssiDbm())) best = r;
        }
        if (best == null) {
            lastError = "no BSS with SSID " + ssidName + " seen";
            return false;
        }
        boolean wantRsn = passphrase != null;
        if (wantRsn != (best.rsn() != null)) {
            lastError = wantRsn ? "network is open but a passphrase was given" : "network requires WPA2";
            return false;
        }
        if (wantRsn && !best.rsn().isWpa2PskCcmp()) {
            lastError = "network does not offer WPA2-PSK/CCMP";
            return false;
        }
        resetLink();
        bssid = best.bssid();
        ssid = want;
        channel = best.channel();
        rsn = wantRsn;
        apRsnIe = best.rsnIe();
        ownRsnIe = rsn ? RsnIe.wpa2PskCcmp().encode() : null;
        pmk = rsn ? Wpa2Crypto.pmk(passphrase, want) : null;
        lastError = null;
        state = State.AUTHENTICATING;
        attempts = 0;
        lastHeardMs = nowMs;
        sendAuth();
        return true;
    }

    /** Sends deauthentication to the AP and drops the link. */
    public void disconnect(int reason, long nowMs) {
        now = nowMs;
        if (bssid != null && state != State.IDLE) {
            out.transmitRadio(Frames.deauth(bssid, mac, bssid, nextSeq(), reason));
        }
        resetLink();
    }

    /** Sends an Ethernet frame (no FCS) through the AP. Returns false if not connected. */
    public boolean sendEthernet(byte[] ethernetFrame, long nowMs) {
        now = nowMs;
        if (state != State.CONNECTED) return false;
        EthernetFrame eth = EthernetFrame.parse(ethernetFrame);
        byte[] mpdu = qosData
                ? new Frame80211(MacHeader.qosData(FrameControl.TO_DS, bssid, mac, eth.dst(), nextSeq(), 0),
                        Llc.encap(eth.etherType(), eth.payload())).encode()
                : Frames.data(FrameControl.TO_DS, bssid, mac, eth.dst(), nextSeq(), eth.etherType(), eth.payload());
        if (rsn) mpdu = Ccmp.encrypt(ptk.tk(), mpdu, ++txPn, 0);
        txFrames++;
        out.transmitRadio(mpdu);
        return true;
    }

    public void tick(long nowMs) {
        now = nowMs;
        if ((state == State.AUTHENTICATING || state == State.ASSOCIATING) && nowMs >= deadline) {
            if (attempts >= MLME_ATTEMPTS) {
                lastError = (state == State.AUTHENTICATING ? "authentication" : "association") + " timed out";
                resetLink();
            } else if (state == State.AUTHENTICATING) {
                sendAuth();
            } else {
                sendAssoc();
            }
        } else if (state == State.HANDSHAKE && nowMs >= deadline) {
            lastError = "4-way handshake timed out";
            disconnect(Mgmt.REASON_4WAY_TIMEOUT, nowMs);
        } else if ((state == State.CONNECTED || state == State.HANDSHAKE) && nowMs - lastHeardMs > BEACON_LOSS_MS) {
            MacAddress lost = bssid;
            disconnect(Mgmt.REASON_INACTIVITY, nowMs);
            lastError = "beacon loss: nothing heard from " + lost + " for " + (nowMs - lastHeardMs) + " ms";
            scan.remove(lost);   // stale: rejoin only after hearing it again
        }
    }

    /**
     * Send every EAPOL-Key frame unencrypted, even after the PTK is installed
     * (what the Rust ecm-wifi station does). Default false: like Linux, EAPOL
     * goes out under the TK once it is installed.
     */
    public void setPlaintextEapol(boolean v) {
        plaintextEapol = v;
    }

    /** Send uplink data as QoS Data (TID 0) instead of plain Data. */
    public void setQosData(boolean v) {
        qosData = v;
    }

    // ------------------------------------------------------------------ status

    public MacAddress mac() { return mac; }
    public State state() { return state; }
    public MacAddress bssid() { return bssid; }
    public int aid() { return aid; }
    public String lastError() { return lastError; }
    public int lastDeauthReason() { return lastReason; }
    public boolean keysInstalled() { return ptk != null; }
    public long rxFrames() { return rxFrames; }
    public long txFrames() { return txFrames; }
    public long replayDrops() { return replayDrops; }
    public long decryptFailures() { return decryptFailures; }
    public long m4Sent() { return m4Sent; }

    /** True if a group key is installed in key slot {@code keyId} (1..3). */
    public boolean hasGroupKey(int keyId) {
        return gtks[keyId & 3] != null;
    }

    // ------------------------------------------------------------------ receive

    public void onReceive(byte[] frame80211, RxMeta meta, long nowMs) {
        now = nowMs;
        if (meta == null) meta = RxMeta.NONE;
        Frame80211 f;
        try {
            f = Frame80211.parse(frame80211);
        } catch (IllegalArgumentException e) {
            return;
        }
        MacHeader h = f.header();
        if (!(h.addr1().equals(mac) || h.addr1().isGroup())) return;
        if (bssid != null && h.addr2() != null && h.addr2().equals(bssid)) lastHeardMs = nowMs;
        try {
            if (h.type() == FrameControl.TYPE_MGMT) handleMgmt(h, f.body(), meta);
            else if (h.type() == FrameControl.TYPE_DATA) handleData(h, frame80211, f.body());
        } catch (IllegalArgumentException e) {
            lastError = "malformed frame: " + e.getMessage();
        }
    }

    private void handleMgmt(MacHeader h, byte[] body, RxMeta meta) {
        switch (h.subtype()) {
            case FrameControl.BEACON, FrameControl.PROBE_RESP -> recordBss(h, Mgmt.Beacon.parse(body), meta);
            case FrameControl.AUTH -> {
                if (state != State.AUTHENTICATING || !h.addr2().equals(bssid)) return;
                Mgmt.Auth a = Mgmt.Auth.parse(body);
                if (a.sequence() != 2 || a.algorithm() != Mgmt.AUTH_OPEN) return;
                if (a.status() != Mgmt.STATUS_SUCCESS) {
                    lastError = "authentication rejected (status " + a.status() + ")";
                    resetLink();
                    return;
                }
                state = State.ASSOCIATING;
                attempts = 0;
                sendAssoc();
            }
            case FrameControl.ASSOC_RESP, FrameControl.REASSOC_RESP -> {
                if (state != State.ASSOCIATING || !h.addr2().equals(bssid)) return;
                Mgmt.AssocResponse r = Mgmt.AssocResponse.parse(body);
                if (r.status() != Mgmt.STATUS_SUCCESS) {
                    lastError = "association rejected (status " + r.status() + ")";
                    resetLink();
                    return;
                }
                aid = r.aid();
                if (rsn) {
                    state = State.HANDSHAKE;
                    deadline = now + HANDSHAKE_TIMEOUT_MS;
                } else {
                    state = State.CONNECTED;
                }
            }
            case FrameControl.DEAUTH, FrameControl.DISASSOC -> {
                if (bssid == null || !h.addr2().equals(bssid) || state == State.IDLE) return;
                lastReason = Mgmt.parseReason(body);
                lastError = (h.subtype() == FrameControl.DEAUTH ? "deauthenticated" : "disassociated")
                        + " by AP (reason " + lastReason + ")";
                resetLink();
            }
            default -> { }
        }
    }

    private void recordBss(MacHeader h, Mgmt.Beacon b, RxMeta meta) {
        MacAddress id = h.addr3();
        byte[] s = b.ssid();
        if (s == null) return;
        ScanResult prev = scan.get(id);
        if (s.length == 0 && prev != null) s = prev.ssid(); // hidden beacon: keep the SSID a probe response revealed
        Ie ds = Ie.find(b.ies(), Ie.DS_PARAMS);
        int ch = ds != null && ds.data().length == 1 ? ds.data()[0] & 0xFF : meta.channel();
        Ie rsnIe = Ie.find(b.ies(), Ie.RSN);
        RsnIe parsed = null;
        if (rsnIe != null) {
            try {
                parsed = RsnIe.parse(rsnIe.data());
            } catch (IllegalArgumentException e) {
                return;
            }
        }
        scan.put(id, new ScanResult(id, s, ch, meta.rssiDbm(), b.capability(), parsed, rsnIe == null ? null : rsnIe.encode(), now));
    }

    private void handleData(MacHeader h, byte[] raw, byte[] body) {
        if (bssid == null || !h.addr2().equals(bssid) || !h.fromDs() || h.toDs()) return;
        if (state == State.IDLE || state == State.AUTHENTICATING || state == State.ASSOCIATING) return;
        boolean group = h.addr1().isGroup();
        boolean decrypted = false;
        if (h.isProtected()) {
            if (!rsn) return;
            int keyId = Ccmp.peekKeyId(raw);
            byte[] key;
            PnReplayWindow window;
            if (group) {
                if (keyId < 1 || gtks[keyId] == null) return;
                key = gtks[keyId];
                window = rxGroupPn[keyId];
            } else {
                if (keyId != 0 || ptk == null) return;
                key = ptk.tk();
                window = rxPn;
            }
            Ccmp.Decrypted d = Ccmp.decrypt(key, raw);
            if (d == null) {
                decryptFailures++;
                return;
            }
            int idx = group || !h.isQos() ? PnReplayWindow.NON_QOS : h.tid();
            if (!window.accept(idx, d.pn())) {
                replayDrops++;
                return;
            }
            body = Arrays.copyOfRange(d.mpdu(), h.length(), d.mpdu().length);
            decrypted = true;
        }
        if (FrameControl.isNullData(h.frameControl())) return;
        Llc.Decap llc = Llc.decap(body);
        if (llc.etherType() == EthernetFrame.ETHERTYPE_EAPOL && !group) {
            handleEapol(llc.payload());
            return;
        }
        if (state != State.CONNECTED || (rsn && !decrypted)) return;
        if (group && h.addr3().equals(mac)) return; // our own broadcast relayed back by the AP
        rxFrames++;
        out.deliverEthernet(new EthernetFrame(h.addr1(), h.addr3(), llc.etherType(), llc.payload()).encode());
    }

    // ------------------------------------------------------------------ supplicant

    private void handleEapol(byte[] pdu) {
        if (!rsn) return;
        EapolKey k = EapolKey.parse(pdu);
        if (k == null || k.descriptorType() != EapolKey.DESC_RSN || k.descriptorVersion() != EapolKey.KI_VERSION_HMAC_SHA1_AES) return;
        if (!k.has(EapolKey.KI_ACK)) return;
        if (lastReplay >= 0 && k.replayCounter() <= lastReplay) return;
        if (k.has(EapolKey.KI_PAIRWISE)) {
            if (!k.has(EapolKey.KI_MIC)) onM1(k);
            else onM3(k);
        } else if (k.has(EapolKey.KI_MIC | EapolKey.KI_SECURE | EapolKey.KI_ENCRYPTED_DATA)) {
            onGroupM1(k);
        }
    }

    private void onM1(EapolKey m1) {
        if (state != State.HANDSHAKE && state != State.CONNECTED) return;
        byte[] an = m1.nonce();
        if (snonce == null || anonce == null || !Arrays.equals(an, anonce)) {
            anonce = an;
            snonce = random(Wpa2Crypto.NONCE_LEN);
        }
        tptk = Wpa2Crypto.derivePtk(pmk, bssid, mac, anonce, snonce);
        EapolKey m2 = EapolKey.of(EapolKey.KI_M2, 0, m1.replayCounter(), snonce, null, ownRsnIe);
        sendEapol(withMic(m2, tptk), false);
    }

    private void onM3(EapolKey m3) {
        if (tptk == null || anonce == null || !k(m3, EapolKey.KI_M3)) return;
        if (!Arrays.equals(m3.nonce(), anonce)) {
            lastError = "M3 ANonce differs from M1";
            return;
        }
        if (!micValid(m3, tptk)) {
            lastError = "M3 MIC mismatch";
            return;
        }
        lastReplay = m3.replayCounter();
        byte[] keyData = AesKeyWrap.unwrap(tptk.kek(), m3.keyData());
        if (keyData == null) {
            lastError = "M3 key data failed to unwrap";
            return;
        }
        byte[] advertised = Kde.findElement(keyData, Ie.RSN);
        if (apRsnIe != null && (advertised == null || !Arrays.equals(advertised, apRsnIe))) {
            lastError = "RSN element in M3 differs from beacon";
            disconnect(Mgmt.REASON_IE_DIFFERENT, now);
            return;
        }
        Kde.Gtk g = Kde.findGtk(keyData);
        if (g == null || g.keyId() == 0 || g.key().length != 16) {
            lastError = "M3 carries no valid GTK";
            return;
        }
        EapolKey m4 = EapolKey.of(EapolKey.KI_M4, 0, m3.replayCounter(), null, null, null);
        boolean reinstall = ptk != null && ptk.equals(tptk);
        sendEapol(withMic(m4, tptk), reinstall);
        m4Sent++;
        if (!reinstall) { // a retransmitted M3 must not reset the PN (no key reinstallation)
            ptk = tptk;
            txPn = 0;
            rxPn.reset(0);
        }
        installGtk(g, EapolKey.pnFromRsc(m3.rsc()));
        state = State.CONNECTED;
        lastError = null;
    }

    private void onGroupM1(EapolKey g1) {
        if (state != State.CONNECTED || ptk == null) return;
        if (!micValid(g1, ptk)) {
            lastError = "group key message 1 MIC mismatch";
            return;
        }
        lastReplay = g1.replayCounter();
        byte[] keyData = AesKeyWrap.unwrap(ptk.kek(), g1.keyData());
        Kde.Gtk g = keyData == null ? null : Kde.findGtk(keyData);
        if (g == null || g.keyId() == 0 || g.key().length != 16) {
            lastError = "group key message 1 carries no valid GTK";
            return;
        }
        installGtk(g, EapolKey.pnFromRsc(g1.rsc()));
        EapolKey g2 = EapolKey.of(EapolKey.KI_G2, 0, g1.replayCounter(), null, null, null);
        sendEapol(withMic(g2, ptk), true);
    }

    private void installGtk(Kde.Gtk g, long rsc) {
        if (gtks[g.keyId()] != null && Arrays.equals(gtks[g.keyId()], g.key())) return; // same key: keep its replay counter
        gtks[g.keyId()] = g.key();
        PnReplayWindow w = new PnReplayWindow();
        w.reset(rsc);
        rxGroupPn[g.keyId()] = w;
    }

    private static boolean k(EapolKey key, int info) {
        return key.keyInfo() == info;
    }

    private static boolean micValid(EapolKey k, Ptk p) {
        return Wpa2Crypto.micEquals(Wpa2Crypto.eapolMic(p.kck(), k.encodeForMic()), k.mic());
    }

    private static byte[] withMic(EapolKey k, Ptk p) {
        return k.withMic(Wpa2Crypto.eapolMic(p.kck(), k.encodeForMic())).encode();
    }

    /** EAPOL to the AP; protected with the installed TK when {@code encrypt} (group handshake, M4 for a repeated M3). */
    private void sendEapol(byte[] pdu, boolean encrypt) {
        byte[] mpdu = Frames.data(FrameControl.TO_DS, bssid, mac, bssid, nextSeq(), EthernetFrame.ETHERTYPE_EAPOL, pdu);
        if (encrypt && !plaintextEapol && ptk != null) mpdu = Ccmp.encrypt(ptk.tk(), mpdu, ++txPn, 0);
        out.transmitRadio(mpdu);
    }

    // ------------------------------------------------------------------ MLME

    private void sendAuth() {
        attempts++;
        deadline = now + MLME_TIMEOUT_MS;
        byte[] body = new Mgmt.Auth(Mgmt.AUTH_OPEN, 1, Mgmt.STATUS_SUCCESS, List.of()).encode();
        out.transmitRadio(Frames.mgmt(FrameControl.AUTH, bssid, mac, bssid, nextSeq(), body));
    }

    private void sendAssoc() {
        attempts++;
        deadline = now + MLME_TIMEOUT_MS;
        List<Ie> ies = new ArrayList<>();
        ies.add(Ie.ssid(ssid));
        List<Ie> rates = Ie.rates(channel);
        ies.add(rates.get(0));
        if (ownRsnIe != null) ies.add(RsnIe.wpa2PskCcmp().toIe());
        if (rates.size() > 1) ies.add(rates.get(1));
        int cap = Mgmt.CAP_ESS | (Channels.is5GHz(channel) ? 0 : Mgmt.CAP_SHORT_SLOT);
        byte[] body = new Mgmt.AssocRequest(cap, 10, null, ies).encode();
        out.transmitRadio(Frames.mgmt(FrameControl.ASSOC_REQ, bssid, mac, bssid, nextSeq(), body));
    }

    private void resetLink() {
        state = State.IDLE;
        aid = 0;
        attempts = 0;
        lastReplay = -1;
        anonce = null;
        snonce = null;
        tptk = null;
        ptk = null;
        txPn = 0;
        rxPn.reset(0);
        Arrays.fill(gtks, null);
        Arrays.fill(rxGroupPn, null);
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
