package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.module.ModuleBays;
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.radio.wifi.RadioWifiContent;
import com.example.evanscomputermod.radio.wifi.WifiModule;
import com.example.evanscomputermod.radio.wifi.mac.LowMac;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.RxMeta;
import com.example.evanscomputermod.radio.wifi80211.ap.AccessPointCore;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import com.example.evanscomputermod.radio.wifi80211.ap.ApOutput;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import net.minecraft.core.BlockPos;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;

import java.nio.file.Files;
import java.util.Arrays;
import java.util.List;
import java.util.Map;
import java.util.SplittableRandom;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.concurrent.ConcurrentLinkedQueue;
import java.util.concurrent.atomic.AtomicInteger;

/**
 * Wi-Fi client scenarios (1.21.1): computers with a Wi-Fi module in a bay
 * ({@code wlan0}), driven with {@code iw}, {@code tcpdump}, {@code wpa_cli} and
 * {@code wpa_supplicant}.
 *
 * <ul>
 *   <li>{@code wifi_monitor}: two computers 8 blocks apart, no access point.
 *       Control: {@code iw dev wlan0 scan} finds nothing. Then {@code a} goes
 *       into monitor mode on channel 1 and captures (to a radiotap pcap and on
 *       screen) the probe requests {@code b}'s scan sends: module, medium,
 *       kernel and program, end to end.</li>
 *   <li>{@code wifi_wpa2_ping}: one computer and a <em>virtual access point</em>
 *       (the 802.11 {@link AccessPointCore} on its own low MAC, standing at the
 *       iron block; the AP block comes in its own lane). Control: no link, so
 *       a ping fails. Then {@code wpa_cli} configures WPA2, {@code wpa_supplicant}
 *       associates and runs the 4-way handshake, and a ping goes through the AP
 *       to a wired gateway it bridges to (192.168.77.1).</li>
 * </ul>
 */
public final class WifiScenarios {
    public static final String PASSPHRASE = "correct horse battery";
    public static final String SSID = "ecm-lab";
    public static final String GATEWAY_IP = "192.168.77.1";
    /** A finished command: the shell prompt is back. */
    static final String PROMPT = "^/\\S* >$";

    private WifiScenarios() {
    }

    // ------------------------------------------------------------ scenarios

    public static Scenario monitor() {
        BlockPos a = new BlockPos(0, 1, 0), b = new BlockPos(8, 1, 0);
        return Scenario.builder("wifi_monitor",
                        "two computers with Wi-Fi modules: an empty scan (control), then monitor mode captures the other's probe requests")
                .host("a", a, "-")
                .host("b", b, "-")
                .decor(new WifiModules(List.of(a, b)))
                .note("Each computer has a Wi-Fi module: the kernel shows wlan0")
                .send("a", "iw dev")
                .expect("a", "^\\s+Interface wlan0$", "a has wlan0")
                .send("b", "iw dev")
                .expect("b", "^\\s+Interface wlan0$", "b has wlan0")
                .note("Control: there is no access point, so a scan finds nothing")
                .send("a", "iw dev wlan0 scan")
                .expect("a", "\\A(?![\\s\\S]*^BSS )[\\s\\S]*" + PROMPT, "the scan finished with no BSS listed")
                .note("Monitor mode on a, channel 1")
                .send("a", "iw dev wlan0 set type monitor")
                .expect("a", PROMPT, "a is in monitor mode", "command failed")
                .send("a", "iw dev wlan0 set channel 1")
                .expect("a", PROMPT, "a listens on channel 1", "command failed")
                .send("a", "iw dev wlan0 info")
                .expect("a", "^\\s+type monitor$", "iw shows type monitor")
                .note("a captures to a radiotap pcap while b scans")
                .send("a", "tcpdump -i wlan0 -c 3 -w probes.pcap")
                .expect("a", "link-type IEEE802_11_RADIO", "tcpdump is listening on wlan0", "No such device|not in monitor mode")
                .send("b", "iw dev wlan0 scan")
                .expect("b", PROMPT, "b's scan finished")
                .expect("a", "^3 packets captured$", "a captured 3 frames from the air")
                .mutate(WifiScenarios::checkProbePcap, "probes.pcap on a holds b's probe requests (radiotap, link type 127)")
                .note("The same capture, printed")
                .send("a", "tcpdump -i wlan0 -c 2")
                .expect("a", "link-type IEEE802_11_RADIO", "tcpdump is listening")
                .send("b", "iw dev wlan0 scan")
                .expect("a", "Probe Request \\(\\) SA:02:", "a printed a probe request from b")
                .timeLimit(60_000)
                .build();
    }

    public static Scenario wpa2Ping() {
        BlockPos pc = new BlockPos(0, 1, 0), ap = new BlockPos(6, 1, 3);
        return Scenario.builder("wifi_wpa2_ping",
                        "a computer joins a WPA2 network (virtual access point) with wpa_supplicant and pings through it")
                .host("pc", pc, "-")
                .decor(new WifiModules(List.of(pc)))
                .decor(new VirtualApDecor(ap))
                .note("Control: no Wi-Fi link yet, so the gateway can't be reached")
                .send("pc", "ifconfig wlan0 192.168.77.2/24")
                .expect("pc", "wlan0: inet 192.168.77.2/24", "wlan0 has 192.168.77.2/24")
                .ping("pc", GATEWAY_IP, 1, 0, "no reply without an association")
                .note("The access point is visible to a scan")
                .send("pc", "iw dev wlan0 scan")
                .expect("pc", "^\\s+SSID: " + SSID + "$", "the scan lists " + SSID)
                .note("Configure WPA2 with wpa_cli and start wpa_supplicant")
                .send("pc", "wpa_cli add_network")
                .expect("pc", "^0$", "network 0 added")
                .send("pc", "wpa_cli set_network 0 ssid " + SSID)
                .expect("pc", "^OK$", "ssid set", "^FAIL")
                .send("pc", "wpa_cli set_network 0 psk \"" + PASSPHRASE + "\"")
                .expect("pc", "^OK$", "passphrase set", "^FAIL")
                .send("pc", "wpa_cli enable_network 0")
                .expect("pc", "^OK$", "network 0 enabled", "^FAIL")
                .send("pc", "wpa_supplicant -B -i wlan0 -c /etc/wpa_supplicant.conf")
                .expect("pc", PROMPT, "wpa_supplicant runs in the background", "wpa_supplicant:")
                .until("pc", "wpa_cli status", "^wpa_state=COMPLETED$", "the 4-way handshake completed")
                .mutate(WifiScenarios::checkApAuthorized, "the access point reports the client AUTHORIZED with the handshake DONE")
                .note("Encrypted traffic both ways through the access point")
                .ping("pc", GATEWAY_IP, 3, 3, "3 of 3 pings answered through the AP")
                .send("pc", "iw dev wlan0 link")
                .expect("pc", "^\\s+tx bitrate: \\d+\\.\\d MBit/s", "iw shows the link's bitrate")
                .send("pc", "wpa_cli list_networks")
                .expect("pc", "^0\\s+" + SSID + "\\s+any\\s+\\[CURRENT\\]$", "network 0 is CURRENT")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ checks

    /** probes.pcap on "a": pcap header with link type 127, three radiotap records, each a probe request from b. */
    static void checkProbePcap(ScenarioRun run) {
        TerminalBlockEntity a = run.terminal("a"), b = run.terminal("b");
        byte[] bMac = moduleOf(b).mac().mac();
        byte[] f;
        try {
            f = Files.readAllBytes(com.example.evanscomputermod.computer.ComputerStorage.path(a).resolve("probes.pcap"));
        } catch (java.io.IOException e) {
            throw new IllegalStateException("probes.pcap missing on a: " + e);
        }
        java.nio.ByteBuffer bb = java.nio.ByteBuffer.wrap(f).order(java.nio.ByteOrder.LITTLE_ENDIAN);
        if (f.length < 24 || bb.getInt(0) != 0xa1b2c3d4 || bb.getInt(20) != 127)
            throw new IllegalStateException("probes.pcap: bad header (" + f.length + " bytes)");
        int off = 24, n = 0;
        while (off + 16 <= f.length) {
            int len = bb.getInt(off + 8);
            int rec = off + 16;
            int rtLen = (f[rec + 2] & 0xff) | (f[rec + 3] & 0xff) << 8;
            int fc = f[rec + rtLen] & 0xff;
            byte[] sa = Arrays.copyOfRange(f, rec + rtLen + 10, rec + rtLen + 16);
            if (f[rec] != 0 || fc != 0x40)
                throw new IllegalStateException("record " + n + ": not a radiotap probe request (fc " + Integer.toHexString(fc) + ")");
            if (!Arrays.equals(sa, bMac))
                throw new IllegalStateException("record " + n + ": SA " + MacAddress.of(sa) + " is not b (" + MacAddress.of(bMac) + ")");
            off = rec + len;
            n++;
        }
        if (n != 3) throw new IllegalStateException("probes.pcap has " + n + " records, expected 3");
    }

    static void checkApAuthorized(ScenarioRun run) {
        VirtualAp ap = VirtualAp.at(run.abs(VirtualApDecor.last));
        if (ap == null) throw new IllegalStateException("virtual AP not running");
        byte[] mac = moduleOf(run.terminal("pc")).mac().mac();
        ClientStatus c = ap.clients().stream().filter(x -> Arrays.equals(x.mac().bytes(), mac)).findFirst().orElse(null);
        if (c == null) throw new IllegalStateException("AP doesn't know the client; clients " + ap.clients());
        if (c.state() != ClientStatus.State.AUTHORIZED || c.handshake() != ClientStatus.Handshake.DONE)
            throw new IllegalStateException("client at the AP: " + c.state() + " / " + c.handshake() + " (" + c.lastError() + ")");
    }

    static WifiModule moduleOf(TerminalBlockEntity t) {
        if (t == null) throw new IllegalStateException("no computer");
        ModuleBays bays = t.getModuleBays();
        for (int i = 0; i < ModuleBays.SLOTS; i++) {
            if (bays.getModule(i) instanceof WifiModule w) return w;
        }
        throw new IllegalStateException("no Wi-Fi module in " + t.getBlockPos());
    }

    // ------------------------------------------------------------ decor

    /** An expansion card and a Wi-Fi module in each listed computer. */
    static final class WifiModules implements Scenario.Decor {
        private final List<BlockPos> computers;

        WifiModules(List<BlockPos> computers) {
            this.computers = computers;
        }

        @Override
        public List<BlockPos> footprint() {
            return List.of();
        }

        @Override
        public void build(ScenarioRun run) {
            for (BlockPos p : computers) {
                if (!(run.level().getBlockEntity(run.abs(p)) instanceof TerminalBlockEntity t))
                    throw new IllegalStateException("no computer at " + run.abs(p));
                ModuleBays bays = t.getModuleBays();
                if (bays.cardCount() == 0) bays.installCard(ModuleBays.LEFT);
                if (bays.getStack(0).isEmpty()) bays.installModule(new ItemStack(RadioWifiContent.WIFI_MODULE.get()), 0);
            }
        }
    }

    /** The virtual access point, marked by an iron block with a lightning rod antenna. */
    static final class VirtualApDecor implements Scenario.Decor {
        static volatile BlockPos last;
        private final BlockPos pos;

        VirtualApDecor(BlockPos pos) {
            this.pos = pos;
        }

        @Override
        public List<BlockPos> footprint() {
            return List.of(pos, pos.above());
        }

        @Override
        public void build(ScenarioRun run) {
            last = pos;
            BlockPos abs = run.abs(pos);
            run.level().setBlock(abs, Blocks.IRON_BLOCK.defaultBlockState(), 3);
            run.level().setBlock(abs.above(), Blocks.LIGHTNING_ROD.defaultBlockState(), 3);
            String dim = run.level().dimension().location().toString();
            VirtualAp.start(abs, Pose.at(dim, abs.getX() + 0.5, abs.getY() + 1.5, abs.getZ() + 0.5));
        }

        @Override
        public void clear(ScenarioRun run) {
            VirtualAp.stop(run.abs(pos));
        }
    }

    // ------------------------------------------------------------ the virtual AP

    /**
     * {@link AccessPointCore} (WPA2-PSK "ecm-lab", channel 6) on a {@link LowMac},
     * driven by its own thread, bridging to a one-host "wired" side that answers
     * ARP and ping for {@link #GATEWAY_IP}. Stops by itself after 5 minutes.
     */
    public static final class VirtualAp implements RadioEndpoint {
        private static final Map<BlockPos, VirtualAp> RUNNING = new ConcurrentHashMap<>();
        static final byte[] BSSID = {0x02, 0x77, 0x00, 0x00, 0x00, 0x01};
        static final byte[] GATEWAY_MAC = {0x02, 0x77, 0x00, 0x00, 0x00, (byte) 0xfe};
        static final byte[] GATEWAY = {(byte) 192, (byte) 168, 77, 1};

        private final UUID id = UUID.randomUUID();
        private final Pose pose;
        private final LowMac mac;
        private final AccessPointCore core;
        private final ConcurrentLinkedQueue<byte[]> wiredIn = new ConcurrentLinkedQueue<>();
        private volatile List<ClientStatus> clients = List.of();
        private volatile List<String> events = List.of();
        private volatile boolean running = true;
        private final RadioMedium medium;
        public final AtomicInteger arpReplies = new AtomicInteger(), pingReplies = new AtomicInteger();

        private VirtualAp(Pose pose, RadioMedium medium) {
            this.pose = pose;
            this.medium = medium;
            this.mac = new LowMac(this, () -> medium, BSSID, 6, LowMac.Options.world(0x77));
            ApOutput out = new ApOutput() {
                @Override
                public void transmitRadio(byte[] frame) {
                    int type = (frame[0] >> 2) & 3;
                    mac.submit(frame, type == 2 ? 24000 : 1000, 200);
                }

                @Override
                public void transmitWired(byte[] eth, MacAddress src) {
                    byte[] reply = wiredReply(eth);
                    if (reply != null) wiredIn.add(reply);
                }
            };
            ApConfig cfg = ApConfig.builder(MacAddress.of(BSSID), SSID).wpa2(PASSPHRASE).channel(6).build();
            this.core = new AccessPointCore(cfg, out, new SplittableRandom(0x5eed));
        }

        public static VirtualAp start(BlockPos key, Pose pose) {
            stop(key);
            RadioMedium m = RadioMediumHooks.medium();
            if (m == null) throw new IllegalStateException("no radio medium running");
            VirtualAp ap = new VirtualAp(pose, m);
            m.register(ap);
            RUNNING.put(key, ap);
            Thread t = new Thread(ap::loop, "ecm-virtual-ap");
            t.setDaemon(true);
            t.start();
            return ap;
        }

        public static VirtualAp at(BlockPos key) {
            return RUNNING.get(key);
        }

        public static void stop(BlockPos key) {
            VirtualAp ap = RUNNING.remove(key);
            if (ap != null) ap.running = false;
        }

        public static void stopAll() {
            for (BlockPos k : List.copyOf(RUNNING.keySet())) stop(k);
        }

        /** Stop every AP; true if any was running (first call after a failure). */
        public static boolean stopAllAndReport() {
            boolean any = !RUNNING.isEmpty();
            for (var e : RUNNING.entrySet()) {
                EvansComputerMod.LOGGER.info("virtual AP {}: clients {}, events {}, arp {}, ping {}", e.getKey(),
                        e.getValue().clients(), e.getValue().events(), e.getValue().arpReplies.get(), e.getValue().pingReplies.get());
            }
            stopAll();
            return any;
        }

        public List<String> events() {
            return events;
        }

        public List<ClientStatus> clients() {
            return clients;
        }

        private void loop() {
            long until = System.currentTimeMillis() + 300_000;
            try {
                while (running && System.currentTimeMillis() < until) {
                    long now = System.currentTimeMillis();
                    for (LowMac.RxFrame f; (f = mac.poll()) != null; ) {
                        core.onReceive(f.frame(), new RxMeta(Math.round(f.rssiDbmX10() / 10f), f.rateKbps(), f.channel()), now);
                    }
                    for (byte[] e; (e = wiredIn.poll()) != null; ) core.onWiredFrame(e);
                    core.tick(now);
                    clients = core.clients();
                    events = core.recentEvents();
                    Thread.sleep(2);
                }
            } catch (InterruptedException ignored) {
                Thread.currentThread().interrupt();
            } catch (RuntimeException e) {
                EvansComputerMod.LOGGER.error("virtual AP failed", e);
            } finally {
                running = false;
                core.shutdown();
                medium.unregister(this);
            }
        }

        /** The wired gateway: ARP replies and ICMP echo replies for 192.168.77.1. */
        byte[] wiredReply(byte[] eth) {
            if (eth.length < 42) return null;
            int type = (eth[12] & 0xff) << 8 | (eth[13] & 0xff);
            if (type == 0x0806 && eth[21] == 1 && Arrays.equals(Arrays.copyOfRange(eth, 38, 42), GATEWAY)) {
                byte[] r = new byte[42];
                System.arraycopy(eth, 6, r, 0, 6);
                System.arraycopy(GATEWAY_MAC, 0, r, 6, 6);
                r[12] = 0x08; r[13] = 0x06;
                System.arraycopy(eth, 14, r, 14, 6);          // htype, ptype, hlen, plen
                r[20] = 0; r[21] = 2;                         // reply
                System.arraycopy(GATEWAY_MAC, 0, r, 22, 6);
                System.arraycopy(GATEWAY, 0, r, 28, 4);
                System.arraycopy(eth, 22, r, 32, 10);         // target = requester's MAC + IP
                arpReplies.incrementAndGet();
                return r;
            }
            if (type == 0x0800 && eth.length >= 34 + 8) {
                int ihl = (eth[14] & 0x0f) * 4;
                int icmp = 14 + ihl;
                if (eth[23] == 1 && Arrays.equals(Arrays.copyOfRange(eth, 30, 34), GATEWAY) && eth.length > icmp + 8
                        && eth[icmp] == 8) {
                    int total = (eth[16] & 0xff) << 8 | (eth[17] & 0xff);
                    byte[] r = Arrays.copyOf(eth, Math.min(eth.length, 14 + total));
                    System.arraycopy(eth, 6, r, 0, 6);
                    System.arraycopy(GATEWAY_MAC, 0, r, 6, 6);
                    System.arraycopy(eth, 26, r, 30, 4);          // dst = requester
                    System.arraycopy(GATEWAY, 0, r, 26, 4);       // src = gateway
                    r[22] = 64;
                    r[24] = 0; r[25] = 0;
                    put16(r, 24, checksum(r, 14, ihl));
                    r[icmp] = 0;                                  // echo reply
                    r[icmp + 2] = 0; r[icmp + 3] = 0;
                    put16(r, icmp + 2, checksum(r, icmp, r.length - icmp));
                    pingReplies.incrementAndGet();
                    return r;
                }
            }
            return null;
        }

        private static int checksum(byte[] b, int off, int len) {
            long sum = 0;
            for (int i = 0; i + 1 < len; i += 2) sum += ((b[off + i] & 0xff) << 8) | (b[off + i + 1] & 0xff);
            if ((len & 1) != 0) sum += (b[off + len - 1] & 0xff) << 8;
            while ((sum >> 16) != 0) sum = (sum & 0xffff) + (sum >> 16);
            return (int) (~sum & 0xffff);
        }

        private static void put16(byte[] b, int off, int v) {
            b[off] = (byte) (v >> 8);
            b[off + 1] = (byte) v;
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return running ? mac.channel() : null; }
        @Override public double maxTxPowerDbm() { return 20; }
        @Override public void onReceive(Reception r) { mac.onReceive(r); }
    }
}
//?}
