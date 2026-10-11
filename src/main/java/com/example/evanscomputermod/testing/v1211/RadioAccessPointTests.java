package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlock;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointContent;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStation;
import com.example.evanscomputermod.radio.wifi.ap.VirtualStations;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ClientStatus;
import com.example.evanscomputermod.radio.wifi80211.frame.EthernetFrame;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.protocol.game.ClientboundBlockEntityDataPacket;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.item.Items;
import net.minecraft.world.level.GameType;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.UUID;

/**
 * Wi-Fi Access Point (lane 3B) in a real world, namespace {@code ecm_radio}.
 * The AP block sits on a cable whose other end is a wired NIC registered with
 * {@code NetworkHub}/{@code CableNetworkManager}; a {@link VirtualStation}
 * (StationCore on the radio medium) is the Wi-Fi client, since the Wi-Fi
 * module lane is still in progress. Every test is bounded by wall clock and
 * cleans up its station, NIC and cable registration.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioAccessPointTests {
    static final String NS = RadioTests.NS;
    static final String STRUCTURE = RadioTests.STRUCTURE;
    static final String PASS = "correct horse battery";
    /** Wall-clock bound of each test's own waits (the backstop is far beyond). */
    static final long LIMIT_MS = 40_000;

    private RadioAccessPointTests() {}

    /** Cable (2..8, 1, 2), AP on top of its east end, the wired NIC's exit at its west end. */
    private static final class Rig {
        final GameTestHelper h;
        final BlockPos apRel = new BlockPos(8, 2, 2);
        final byte[] wired = {0x02, 0x7e, 0, 0, 0, 0};
        final NetworkHub hub = NetworkHub.getInstance();
        final CableNetworkManager cables = CableNetworkManager.getInstance();
        final long startMs = System.currentTimeMillis();
        AccessPointBlockEntity ap;
        VirtualStation sta;
        String staKey;

        Rig(GameTestHelper h, String tag) {
            this.h = h;
            long r = UUID.randomUUID().getLeastSignificantBits();
            for (int i = 2; i < 6; i++) wired[i] = (byte) (r >>> (8 * i));
            for (int x = 2; x <= 8; x++) h.setBlock(new BlockPos(x, 1, 2), ModBlocks.NETWORK_CABLE.get().defaultBlockState());
            for (int x = 2; x <= 8; x++) {
                BlockPos p = h.absolutePos(new BlockPos(x, 1, 2));
                BlockState s = h.getLevel().getBlockState(p);
                for (Direction d : Direction.values()) {
                    BlockPos q = p.relative(d);
                    s = s.setValue(NetworkCableBlock.getPropertyForDirection(d),
                            NetworkCableBlock.canConnectToFace(h.getLevel().getBlockState(q), d.getOpposite(), h.getLevel(), q));
                }
                h.getLevel().setBlock(p, s, 3);
            }
            h.setBlock(apRel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
            ap = (AccessPointBlockEntity) h.getBlockEntity(apRel);
            hub.registerNic(wired, (irq, payload) -> {});
            cables.registerTerminal(h.absolutePos(new BlockPos(1, 1, 2)), h.getLevel().dimension(), new byte[][] {wired},
                    new BlockPos[] {h.absolutePos(new BlockPos(2, 1, 2))});
            staKey = "test/" + tag + "/" + UUID.randomUUID();
        }

        void station(String ssid, String pass, int distance) {
            BlockPos p = h.absolutePos(apRel.offset(0, 0, distance));
            MacAddress mac = new MacAddress(0x025A_0000_0000L | (UUID.randomUUID().getLeastSignificantBits() & 0xFFFF_FFFFL));
            sta = VirtualStations.start(staKey, new VirtualStation(mac,
                    Pose.at(h.getLevel().dimension().location().toString(), p.getX() + 0.5, p.getY() + 1, p.getZ() + 0.5),
                    6, ssid, pass, mac.value()));
        }

        boolean timedOut() {
            return System.currentTimeMillis() - startMs > LIMIT_MS;
        }

        /** Next frame the wired NIC received from {@code src} (draining others), or null. */
        byte[] wiredFrom(MacAddress src) {
            byte[] f;
            while ((f = hub.receive(wired)) != null) {
                if (f.length >= 14 && MacAddress.read(f, 6).equals(src)) return f;
            }
            return null;
        }

        void close() {
            VirtualStations.stop(staKey);
            cables.unregisterTerminal(new byte[][] {wired});
            hub.unregisterNic(wired);
        }
    }

    private static ApSettings settings(Security sec) {
        return new ApSettings("ecm-test", false, sec, 6, 20, false, null, null);
    }

    /**
     * WPA2 end to end: a station associates over the medium and completes the
     * 4-way handshake; its ARP broadcast reaches the wired NIC with its own MAC
     * as source (learned behind the AP's port); a unicast from the wired NIC to
     * its MAC arrives at the station decrypted.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ap_wpa2")
    public static void access_point_bridges_wpa2_station(GameTestHelper h) {
        Rig rig = new Rig(h, "wpa2");
        String[] failure = {null};
        String err = rig.ap.applySettings(settings(Security.WPA2_PSK), PASS);
        if (err != null) failure[0] = "settings rejected: " + err;
        int[] step = {0};
        byte[] marker = "hello over the air".getBytes(StandardCharsets.US_ASCII);
        TestDriver.drive(h, NS, "access_point_bridges_wpa2_station", () -> {
            if (rig.timedOut()) {
                failure[0] = "timed out at step " + step[0] + "; station " + (rig.sta == null ? "-" : rig.sta.core().state() + " "
                        + rig.sta.core().lastError()) + "; AP events " + (rig.ap.core() == null ? "radio off" : rig.ap.core().recentEvents());
                rig.close();
                return false;
            }
            switch (step[0]) {
                case 0 -> {
                    if (rig.ap.core() == null || !rig.ap.cabled()) return false;
                    rig.station("ecm-test", PASS, 10);
                    step[0] = 1;
                }
                case 1 -> {
                    if (!rig.sta.connected()) return false;
                    ClientStatus c = rig.ap.core().client(rig.sta.mac());
                    if (c == null || c.state() != ClientStatus.State.AUTHORIZED || c.handshake() != ClientStatus.Handshake.DONE) {
                        failure[0] = "station connected but AP says " + c;
                        return false;
                    }
                    while (rig.hub.receive(rig.wired) != null) { /* drain */ }
                    byte[] arp = new EthernetFrame(MacAddress.BROADCAST, rig.sta.mac(), EthernetFrame.ETHERTYPE_ARP, new byte[28]).encode();
                    if (!rig.sta.sendEthernet(arp)) failure[0] = "station refused to send";
                    step[0] = 2;
                }
                case 2 -> {
                    byte[] f = rig.wiredFrom(rig.sta.mac());
                    if (f == null) return false;
                    if (f[0] != (byte) 0xFF || ((f[12] & 0xFF) << 8 | (f[13] & 0xFF)) != EthernetFrame.ETHERTYPE_ARP) {
                        failure[0] = "wired NIC got a different frame from the station: " + Arrays.toString(Arrays.copyOf(f, 14));
                        return false;
                    }
                    if (!Arrays.equals(rig.hub.bridgePortOf(rig.sta.mac().bytes()), rig.ap.portMac())) {
                        failure[0] = "station MAC not learned behind the AP's bridge port";
                        return false;
                    }
                    rig.hub.transmit(rig.wired, new EthernetFrame(rig.sta.mac(), MacAddress.of(rig.wired),
                            EthernetFrame.ETHERTYPE_IPV4, marker).encode());
                    step[0] = 3;
                }
                default -> {
                    boolean got = rig.sta.delivered().stream().anyMatch(e -> {
                        EthernetFrame ef = EthernetFrame.parse(e);
                        return ef.src().equals(MacAddress.of(rig.wired)) && Arrays.equals(Arrays.copyOf(ef.payload(), marker.length), marker);
                    });
                    if (!got) return false;
                    if (rig.sta.core().decryptFailures() != 0) {
                        failure[0] = "station had " + rig.sta.core().decryptFailures() + " CCMP decrypt failures";
                        return false;
                    }
                    rig.close();
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /**
     * Controls: a station with the wrong passphrase reaches the 4-way handshake
     * but never gets authorized (the AP logs the M2 MIC mismatch), cannot send,
     * and nothing from its MAC reaches the cable.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ap_wrongpass")
    public static void access_point_rejects_wrong_passphrase(GameTestHelper h) {
        Rig rig = new Rig(h, "wrong");
        String[] failure = {null};
        String err = rig.ap.applySettings(settings(Security.WPA2_PSK), PASS);
        if (err != null) failure[0] = "settings rejected: " + err;
        int[] step = {0};
        long[] micAt = {-1};
        TestDriver.drive(h, NS, "access_point_rejects_wrong_passphrase", () -> {
            if (rig.timedOut()) {
                failure[0] = "timed out at step " + step[0] + "; AP events " + (rig.ap.core() == null ? "radio off" : rig.ap.core().recentEvents());
                rig.close();
                return false;
            }
            switch (step[0]) {
                case 0 -> {
                    if (rig.ap.core() == null || !rig.ap.cabled()) return false;
                    rig.station("ecm-test", "this is not it", 10);
                    step[0] = 1;
                }
                default -> {
                    if (rig.sta.everConnected()) {
                        failure[0] = "station connected with the wrong passphrase";
                        return false;
                    }
                    ClientStatus c = rig.ap.core().client(rig.sta.mac());
                    if (c != null && c.state() == ClientStatus.State.AUTHORIZED) {
                        failure[0] = "AP authorized the wrong-passphrase station";
                        return false;
                    }
                    boolean mic = rig.ap.core().recentEvents().stream().anyMatch(e -> e.contains(rig.sta.mac().toString()) && e.contains("MIC"));
                    if (mic && micAt[0] < 0) {
                        micAt[0] = h.getTick();
                        byte[] arp = new EthernetFrame(MacAddress.BROADCAST, rig.sta.mac(), EthernetFrame.ETHERTYPE_ARP, new byte[28]).encode();
                        if (rig.sta.sendEthernet(arp)) failure[0] = "station could send data without keys";
                    }
                    if (micAt[0] < 0 || h.getTick() - micAt[0] < 60) return false;   // keep watching for 3 s of game time
                    if (rig.wiredFrom(rig.sta.mac()) != null) {
                        failure[0] = "a frame from the unauthorized station reached the cable";
                        return false;
                    }
                    rig.close();
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /** An open network: association alone authorizes, and the station's broadcast reaches the cable. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ap_open")
    public static void access_point_open_network_bridges(GameTestHelper h) {
        Rig rig = new Rig(h, "open");
        String[] failure = {null};
        String err = rig.ap.applySettings(settings(Security.OPEN), "");
        if (err != null) failure[0] = "settings rejected: " + err;
        int[] step = {0};
        TestDriver.drive(h, NS, "access_point_open_network_bridges", () -> {
            if (rig.timedOut()) {
                failure[0] = "timed out at step " + step[0];
                rig.close();
                return false;
            }
            switch (step[0]) {
                case 0 -> {
                    if (rig.ap.core() == null || !rig.ap.cabled()) return false;
                    rig.station("ecm-test", null, 12);
                    step[0] = 1;
                }
                case 1 -> {
                    if (!rig.sta.connected()) return false;
                    while (rig.hub.receive(rig.wired) != null) { /* drain */ }
                    rig.sta.sendEthernet(new EthernetFrame(MacAddress.BROADCAST, rig.sta.mac(), EthernetFrame.ETHERTYPE_ARP, new byte[28]).encode());
                    step[0] = 2;
                }
                default -> {
                    if (rig.wiredFrom(rig.sta.mac()) == null) return false;
                    rig.close();
                    return true;
                }
            }
            return false;
        }, () -> failure[0]);
    }

    /**
     * The passphrase is saved server-side but is in neither the block-entity
     * sync tag, the sync packet, nor the GUI view the client gets; it survives
     * a save/load on the server.
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ap_secret")
    public static void access_point_passphrase_stays_on_server(GameTestHelper h) {
        String secret = "zebra-quokka-SECRET-42";
        BlockPos rel = new BlockPos(4, 2, 4);
        h.setBlock(rel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        var be = (AccessPointBlockEntity) h.getBlockEntity(rel);
        String[] failure = {null};
        String err = be.applySettings(settings(Security.WPA2_PSK), secret);
        var regs = h.getLevel().registryAccess();
        if (err != null) failure[0] = "settings rejected: " + err;
        else {
            CompoundTag update = be.getUpdateTag(regs);
            ClientboundBlockEntityDataPacket pkt = (ClientboundBlockEntityDataPacket) be.getUpdatePacket();
            FriendlyByteBuf view = new FriendlyByteBuf(Unpooled.buffer());
            ApPackets.writeView(view, be.view(null));
            String viewBytes = view.toString(StandardCharsets.ISO_8859_1);
            CompoundTag saved = be.saveWithFullMetadata(regs);
            if (update.toString().contains(secret) || update.contains(AccessPointBlockEntity.PASSPHRASE_KEY))
                failure[0] = "passphrase in the block-entity update tag: " + update;
            else if (pkt.getTag().toString().contains(secret))
                failure[0] = "passphrase in the block-entity data packet";
            else if (viewBytes.contains(secret))
                failure[0] = "passphrase in the GUI view packet";
            else if (!update.getBoolean("HasPassphrase") || !"WPA2_PSK".equals(update.getString("Security")))
                failure[0] = "update tag lacks the public settings: " + update;
            else if (!secret.equals(saved.getString(AccessPointBlockEntity.PASSPHRASE_KEY)))
                failure[0] = "control: passphrase not in the server save";
            else {
                // Reload from the save: the core comes back up on WPA2 with the stored passphrase.
                var copy = new AccessPointBlockEntity(be.getBlockPos(), be.getBlockState());
                copy.loadWithComponents(saved, regs);
                CompoundTag again = copy.saveWithFullMetadata(regs);
                if (!secret.equals(again.getString(AccessPointBlockEntity.PASSPHRASE_KEY)) || copy.settings().security() != Security.WPA2_PSK)
                    failure[0] = "passphrase or settings lost on reload";
                // A client-side copy loaded from the sync tag knows only that a passphrase exists.
                var client = new AccessPointBlockEntity(be.getBlockPos(), be.getBlockState());
                client.loadWithComponents(update, regs);
                if (failure[0] == null && (client.hasPassphrase() || client.saveWithFullMetadata(regs).toString().contains(secret)))
                    failure[0] = "client copy holds the passphrase";
            }
        }
        var cap = h.getLevel().getCapability(com.example.evanscomputermod.radio.api.RadioCapabilities.ENDPOINT, h.absolutePos(rel), null);
        if (failure[0] == null && cap != be.endpoint()) failure[0] = "RadioCapabilities.ENDPOINT is not the AP's antenna: " + cap;
        TestDriver.drive(h, NS, "access_point_passphrase_stays_on_server",() -> failure[0] == null, () -> failure[0]);
    }

    /**
     * Configuration needs the owner (the placer), an op or creative mode, and
     * a nearby player; a stranger's Configure packet is refused and changes
     * nothing. Sneak + wrench factory-resets for the same players (open network,
     * passphrase erased, the owner stays the owner); a stranger's wrench is refused. A non-wrench item is not a wrench (control).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".ap_owner")
    public static void access_point_owner_config_and_wrench_reset(GameTestHelper h) {
        BlockPos rel = new BlockPos(4, 2, 4);
        h.setBlock(rel, AccessPointContent.ACCESS_POINT.get().defaultBlockState());
        var be = (AccessPointBlockEntity) h.getBlockEntity(rel);
        String[] failure = {null};
        // Mock players (not ServerPlayers: logging those in trips Sable's datapack sync in the test server).
        Player owner = h.makeMockPlayer(GameType.SURVIVAL);
        Player stranger = h.makeMockPlayer(GameType.SURVIVAL);
        Player creative = h.makeMockPlayer(GameType.CREATIVE);
        BlockPos abs = h.absolutePos(rel);
        owner.moveTo(abs.getX() + 2.5, abs.getY(), abs.getZ() + 0.5);
        stranger.moveTo(abs.getX() - 1.5, abs.getY(), abs.getZ() + 0.5);
        be.claim(owner);
        try {
            ApSettings wpa = settings(Security.WPA2_PSK);
            String r1 = ApPackets.configure(stranger, new ApPackets.Configure(abs, ApPackets.Action.APPLY, wpa, PASS, 0));
            if (!r1.startsWith("Only") || be.settings().security() != Security.OPEN)
                failure[0] = "stranger could configure: " + r1;
            else if (be.canConfigure(stranger))
                failure[0] = "stranger passes canConfigure";
            else {
                String r2 = ApPackets.configure(owner, new ApPackets.Configure(abs, ApPackets.Action.APPLY, wpa, PASS, 0));
                if (!"Settings applied".equals(r2) || be.settings().security() != Security.WPA2_PSK || !be.hasPassphrase())
                    failure[0] = "owner's settings not applied: " + r2;
                String r3 = ApPackets.configure(owner, new ApPackets.Configure(abs, ApPackets.Action.APPLY,
                        new ApSettings("x", false, Security.WPA2_PSK, 6, 99, false, null, null), "", 0));
                if (failure[0] == null && (!r3.startsWith("Not applied") || be.settings().txPowerDbm() != 20))
                    failure[0] = "invalid settings were applied: " + r3;
                if (failure[0] == null && !be.canConfigure(creative)) failure[0] = "creative player can't configure";
                creative.moveTo(abs.getX() + 20.5, abs.getY(), abs.getZ() + 0.5);
                String r4 = ApPackets.configure(creative, new ApPackets.Configure(abs, ApPackets.Action.APPLY, wpa, PASS, 0));
                if (failure[0] == null && !r4.startsWith("Too far")) failure[0] = "a player 20 blocks away could configure: " + r4;
                if (failure[0] == null && AccessPointBlock.isWrench(new ItemStack(Items.STICK)))
                    failure[0] = "control: a stick counts as a wrench";
                if (failure[0] == null && (be.factoryReset(stranger) || be.settings().security() != Security.WPA2_PSK))
                    failure[0] = "a stranger's wrench reset the AP: " + be.settings();
                if (failure[0] == null) {
                    be.factoryReset(owner);
                    if (be.settings().security() != Security.OPEN || be.hasPassphrase() || !owner.getUUID().equals(be.owner()))
                        failure[0] = "factory reset incomplete: " + be.settings() + " pass=" + be.hasPassphrase() + " owner " + be.owner();
                }
            }
        } catch (RuntimeException e) {
            failure[0] = "threw " + e;
        }
        TestDriver.drive(h, NS, "access_point_owner_config_and_wrench_reset", () -> failure[0] == null, () -> failure[0]);
    }

    /** The {@code wifi_room} scenario as spawned by {@code /ecm scenario spawn}: the pc pings the WPA2 phone; the rogue gets nothing. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".wifi_room")
    public static void wifi_room_scenario(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("wifi_room"), true);
    }

}
//?}
