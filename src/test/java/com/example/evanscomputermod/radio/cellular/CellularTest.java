package com.example.evanscomputermod.radio.cellular;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;

import org.junit.jupiter.api.Test;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;

/** Cellular PHY math, frames and the tower/modem engines on the reference medium. */
public class CellularTest {

    // ------------------------------------------------------------ PHY

    @Test
    void channelPlansSitInsideTheirBandsAndAvoidTheGaps() {
        for (int n = 1; n <= CellTech.LTE.channels; n++) {
            for (Channel c : List.of(CellTech.LTE.downlink(n), CellTech.LTE.uplink(n))) {
                assertEquals(Band.UHF, c.band(), "LTE ch " + n);
                assertTrue(c.lowHz() >= 690e6 && c.highHz() <= 900e6, "LTE ch " + n + " at " + c.centerHz());
            }
        }
        for (int n = 1; n <= CellTech.MMWAVE.channels; n++) {
            for (Channel c : List.of(CellTech.MMWAVE.downlink(n), CellTech.MMWAVE.uplink(n))) {
                assertEquals(Band.MICROWAVE, c.band(), "mmWave ch " + n);
                assertTrue(c.centerHz() > 27e9 && c.centerHz() < 29e9);
            }
        }
        for (CellTech t : CellTech.values()) {
            // Uplink and downlink of every channel never overlap any downlink (FDD duplex gap).
            for (int a = 1; a <= t.channels; a++)
                for (int b = 1; b <= t.channels; b++) assertFalse(t.uplink(a).overlaps(t.downlink(b)), t + " " + a + "/" + b);
            // Slices tile the uplink exactly, without overlapping each other.
            double total = 0;
            for (int s = 0; s < t.ulSlices; s++) {
                Channel c = t.uplinkSlice(1, s);
                total += c.bandwidthHz();
                assertTrue(c.lowHz() >= t.uplink(1).lowHz() - 1 && c.highHz() <= t.uplink(1).highHz() + 1);
                if (s > 0) assertFalse(c.overlaps(t.uplinkSlice(1, s - 1)));
            }
            assertEquals(t.bandwidthHz, total, 1);
            assertEquals(3, t.channelOfDownlink(t.downlink(3).centerHz()));
        }
    }

    @Test
    void cqiTableAndRates() {
        for (int c = 2; c <= CellPhy.MAX_CQI; c++) {
            assertTrue(CellPhy.efficiency(c) > CellPhy.efficiency(c - 1));
            assertTrue(CellPhy.requiredSinrDb(c) > CellPhy.requiredSinrDb(c - 1));
        }
        // LTE 10 MHz at 64QAM CQI 15: 5.5547 b/s/Hz * 10 MHz * 0.75 = 41.7 Mbit/s.
        assertEquals(41.66e6, CellPhy.downlinkRateBps(CellTech.LTE, 15), 0.05e6);
        // NR 100 MHz at 256QAM CQI 18: 7.4063 * 100 MHz * 0.82 = 607 Mbit/s.
        assertEquals(607.3e6, CellPhy.downlinkRateBps(CellTech.MMWAVE, 18), 0.5e6);
        // LTE stops at CQI 15 however good the signal; mmWave goes to 18.
        assertEquals(15, CellPhy.cqiFor(CellTech.LTE, 60));
        assertEquals(18, CellPhy.cqiFor(CellTech.MMWAVE, 60));
        // Link adaptation keeps a 3 dB margin: 64QAM CQI 10 needs 11.7 dB, so 14.6 dB is not yet enough.
        assertEquals(9, CellPhy.cqiFor(CellTech.LTE, 14.6));
        assertEquals(10, CellPhy.cqiFor(CellTech.LTE, 14.7));
        assertEquals(CellPhy.ROBUST_CQI, CellPhy.cqiFor(CellTech.LTE, -20));
        assertEquals(CellPhy.ROBUST_CQI, CellPhy.cqiFor(CellTech.LTE, Double.NaN));
        // The medium sees the same thresholds through the modulation name.
        assertEquals(5.9, BasicRadioMedium.requiredSinrDb(CellPhy.modulation(7)), 1e-9);
        assertEquals(-6.7, BasicRadioMedium.requiredSinrDb("CELL-CQI1"), 1e-9);
        // A 1500-byte frame at LTE CQI 15 is 288 us plus a 71 us symbol.
        assertEquals(71 + 288, CellPhy.airtimeUs(CellTech.LTE, 1500, CellPhy.downlinkRateBps(CellTech.LTE, 15)), 1);
    }

    @Test
    void linkBudgets() {
        // Noise in 10 MHz with a 7 dB NF: -174 + 70 + 7 = -97 dBm; sensitivity at the robust CQI 7.7 dB under it.
        assertEquals(-97, CellPhy.noiseDbm(10e6, 7), 1e-9);
        assertEquals(-104.7, CellPhy.ueSensitivityDbm(CellTech.LTE), 1e-9);
        // LTE downlink 300 m in free space: 43 dBm + 14 dBi + 2.15 dBi - 80.1 dB = -21 dBm: top CQI.
        double fs = BasicRadioMedium.freeSpaceLossDb(300, CellTech.LTE.downlink(1).centerHz());
        assertEquals(80.0, fs, 0.5);
        double rx = CellTech.LTE.towerTxDbm + CellTech.LTE.towerPanelGainDbi + 2.15 - fs;
        double sinr = rx - CellPhy.noiseDbm(CellTech.LTE.bandwidthHz, CellTech.LTE.ueNoiseFigureDb);
        assertEquals(15, CellPhy.cqiFor(CellTech.LTE, sinr));
        // Reciprocity: the uplink estimate from that beacon also clears 64QAM on a 2.5 MHz slice.
        double ul = CellPhy.uplinkSinrEstimateDb(CellTech.LTE, rx - CellTech.LTE.towerPanelGainDbi * 0, CellTech.LTE.towerTxDbm);
        assertTrue(ul > 30, "uplink SINR " + ul);
        // mmWave free space at 28 GHz is ~31 dB worse than LTE at 800 MHz for the same distance.
        double mm = BasicRadioMedium.freeSpaceLossDb(300, CellTech.MMWAVE.downlink(1).centerHz());
        assertEquals(31, mm - fs, 0.5);
    }

    @Test
    void antennaPatterns() {
        var lte = CellAntennas.tower(CellAntennas.panelFor(CellTech.LTE), List.of(new double[] {0, 1}), 0);   // facing south (+Z)
        assertEquals(14 - 12 * Math.pow(3 / 20.0, 2), lte.gainDbi(0, 0, 1), 0.01);       // boresight, 3 deg below the tilt
        assertTrue(lte.gainDbi(0, 0, 1) - lte.gainDbi(1, 0, 0) > 10, "65 degree sector");
        assertEquals(14 - 25, lte.gainDbi(0, 0, -1), 0.01);                               // front-to-back
        var four = CellAntennas.tower(CellAntennas.panelFor(CellTech.LTE),
                List.of(new double[] {0, 1}, new double[] {1, 0}, new double[] {0, -1}, new double[] {-1, 0}), 0);
        assertTrue(four.gainDbi(0, 0, -1) > 13, "four panels cover all round");
        var mm = CellAntennas.tower(CellAntennas.panelFor(CellTech.MMWAVE), List.of(new double[] {1, 0}), 0);
        assertEquals(20, mm.gainDbi(1, 0, 0), 0.01);
        assertTrue(mm.gainDbi(Math.cos(Math.toRadians(50)), 0, Math.sin(Math.toRadians(50))) > 16, "steered 50 degrees off");
        assertEquals(-5, mm.gainDbi(-1, 0, 0), 0.01);
        var none = CellAntennas.tower(CellAntennas.panelFor(CellTech.LTE), List.of(), 0);
        assertTrue(none.peakGainDbi() < -50);
    }

    @Test
    void framesRoundTrip() {
        UUID tower = UUID.randomUUID();
        int cell = CellFrame.cellKey(tower);
        var b = (CellFrame.Beacon) CellFrame.decode(CellFrame.beacon(cell, CellTech.MMWAVE, 3, 30, 2, tower, "red-mobile"));
        assertEquals(cell, b.cellKey());
        assertEquals(CellTech.MMWAVE, b.tech());
        assertEquals(3, b.channel());
        assertEquals(30, b.txPowerDbm(), 1e-9);
        assertEquals(tower, b.towerId());
        assertEquals("red-mobile", b.carrier());
        byte[] mac = {2, 1, 2, 3, 4, 5};
        var a = (CellFrame.Attach) CellFrame.decode(CellFrame.attach(cell, 310_001_000_000_007L, mac, 99, Double.NaN, 1, new byte[8]));
        assertEquals(310_001_000_000_007L, a.imsi());
        assertArrayEquals(mac, a.mac());
        assertTrue(Double.isNaN(a.dlSinrDb()));
        assertTrue(a.handover());
        var d = (CellFrame.Data) CellFrame.decode(CellFrame.dataUplink(cell, 7, 12.3, new byte[] {1, 2, 3}));
        assertTrue(d.uplink());
        assertEquals(12.3, d.dlSinrDb(), 1e-9);
        assertArrayEquals(new byte[] {1, 2, 3}, d.ethernet());
        assertNull(CellFrame.decode(new byte[] {CellFrame.ACCEPT, 0, 0, 0, 1, 2}));   // truncated
        assertNull(CellFrame.decode(new byte[] {42, 0, 0, 0, 1}));
    }

    @Test
    void simProofs() {
        SimCard s = new SimCard("red", 310_001_000_000_001L, SimCard.newKeyHex(new SecureRandom()));
        byte[] mac = {2, 0, 0, 0, 0, 1};
        byte[] p = SimCard.attachProof(s.key(), 5, s.imsi(), mac, 77);
        assertTrue(SimCard.proofEquals(p, SimCard.attachProof(s.key(), 5, s.imsi(), mac, 77)));
        assertFalse(SimCard.proofEquals(p, SimCard.attachProof(s.key(), 5, s.imsi(), mac, 78)));
        SimCard other = new SimCard("red", s.imsi(), SimCard.newKeyHex(new SecureRandom()));
        assertFalse(SimCard.proofEquals(p, SimCard.attachProof(other.key(), 5, s.imsi(), mac, 77)));
        assertEquals("310001000000001", s.imsiText());
        assertNull(SimCard.normaliseCarrier("Bad Name!"));
        assertEquals("red-mobile", SimCard.normaliseCarrier(" Red-Mobile "));
    }

    // ------------------------------------------------------------ engines

    final AtomicLong clock = new AtomicLong(1_000_000);
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 7);

    /** The carrier's subscriber table. */
    static final class FakeCore implements TowerLink.Core {
        final Map<Long, String> carrierOf = new HashMap<>();
        final Map<Long, byte[]> keys = new HashMap<>();
        final Map<Long, Long> quota = new HashMap<>(), used = new HashMap<>();
        final List<TowerLink> towers = new ArrayList<>();

        SimCard issue(String carrier, long imsi) {
            SimCard s = new SimCard(carrier, imsi, SimCard.newKeyHex(new SecureRandom()));
            carrierOf.put(imsi, carrier);
            keys.put(imsi, s.key());
            return s;
        }

        @Override public Decision authorize(String carrier, long imsi) {
            if (!carrier.equals(carrierOf.get(imsi))) return Decision.reject(CellFrame.Cause.UNKNOWN_SIM);
            Long q = quota.get(imsi);
            if (q != null && used.getOrDefault(imsi, 0L) >= q) return Decision.reject(CellFrame.Cause.QUOTA);
            return new Decision(keys.get(imsi), CellFrame.Cause.NONE);
        }

        @Override public void attached(TowerLink tower, long imsi) {
            for (TowerLink t : towers) if (t != tower) t.dropDevice(imsi);
        }

        @Override public void detached(TowerLink tower, long imsi) {}

        @Override public boolean account(String carrier, long imsi, long bytes) {
            long u = used.merge(imsi, bytes, Long::sum);
            Long q = quota.get(imsi);
            return q == null || u < q;
        }
    }

    final FakeCore core = new FakeCore();

    final class Tower {
        final TowerLink link;
        final List<byte[]> cable = new CopyOnWriteArrayList<>();

        Tower(CellTech tech, String carrier, int channel, double x, double y, double z) {
            link = new TowerLink(UUID.randomUUID(), tech, () -> medium, core, cable::add);
            link.configure(carrier, channel);
            link.setAntenna(Pose.at("overworld", x, y, z), CellAntennas.tower(CellAntennas.panelFor(tech),
                    List.of(new double[] {0, 1}, new double[] {1, 0}, new double[] {0, -1}, new double[] {-1, 0}), 0));
            core.towers.add(link);
        }
    }

    static byte[] eth(byte[] dst, byte[] src, int tag) {
        byte[] f = new byte[60];
        System.arraycopy(dst, 0, f, 0, 6);
        System.arraycopy(src, 0, f, 6, 6);
        f[12] = 0x08;
        f[20] = (byte) tag;
        return f;
    }

    static final byte[] SERVER = {0x02, 0x55, 0, 0, 0, 1}, BCAST = {-1, -1, -1, -1, -1, -1};

    ModemLink modem(CellTech tech, int n, double x, double y, double z) {
        ModemLink m = new ModemLink(UUID.randomUUID(), tech, new byte[] {0x02, 0x66, 0, 0, 0, (byte) n}, () -> medium);
        m.setPose(Pose.at("overworld", x, y, z));
        return m;
    }

    /** Advance the world by {@code ticks} 50 ms server ticks. */
    void run(int ticks, List<Tower> towers, ModemLink... modems) {
        for (int i = 0; i < ticks; i++) {
            clock.addAndGet(50_000);
            for (Tower t : towers) t.link.tick();
            for (ModemLink m : modems) m.tick();
        }
    }

    @Test
    void attachCarryFramesAndControls() {
        Tower t = new Tower(CellTech.LTE, "red", 4, 0, 20, 0);
        ModemLink m = modem(CellTech.LTE, 1, 300, 2, 0);
        // No SIM: nothing happens.
        run(20, List.of(t), m);
        assertEquals(ModemLink.State.NO_SIM, m.state());
        // Wrong carrier: it hears the tower but never asks it.
        m.setSim(core.issue("blue", 310_002_000_000_001L));
        run(60, List.of(t), m);
        assertEquals(ModemLink.State.SEARCHING, m.state());
        assertTrue(m.cells().stream().anyMatch(c -> c.carrier().equals("red")), "heard the red tower: " + m.cells());
        assertEquals(0, t.link.attaches() + t.link.rejects(), "never asked the other carrier's tower");
        // A forged SIM (right carrier and IMSI, wrong key) is refused.
        SimCard real = core.issue("red", 310_001_000_000_001L);
        m.setSim(new SimCard("red", real.imsi(), SimCard.newKeyHex(new SecureRandom())));
        run(60, List.of(t), m);
        assertEquals(ModemLink.State.REFUSED, m.state(), m.message());
        assertTrue(m.message().contains("authentication"), m.message());
        // The real SIM attaches (it scans 6 channels, 300 ms each, to find channel 4).
        m.setSim(real);
        run(60, List.of(t), m);
        assertEquals(ModemLink.State.CONNECTED, m.state(), m.message() + " " + m.events());
        assertEquals(1, t.link.deviceCount());
        assertEquals(15, m.downlinkCqi(), "300 m in free space is top quality");
        // Uplink reaches the cable, downlink reaches the modem; broadcasts go both ways.
        assertTrue(m.sendEthernet(eth(SERVER, m.mac(), 1)));
        run(1, List.of(t), m);
        assertEquals(1, t.cable.size());
        assertEquals(1, t.cable.get(0)[20]);
        t.link.fromCable(eth(m.mac(), SERVER, 2));
        t.link.fromCable(eth(new byte[] {2, 0x55, 9, 9, 9, 9}, SERVER, 3));   // someone else: not sent down
        t.link.fromCable(eth(BCAST, SERVER, 4));
        run(1, List.of(t), m);
        byte[] a = m.pollEthernet(), b = m.pollEthernet();
        assertNotNull(a);
        assertNotNull(b);
        assertNull(m.pollEthernet());
        assertEquals(2, a[20]);
        assertEquals(4, b[20]);
        // Quota: once used up, the tower releases the SIM and refuses it again.
        core.quota.put(real.imsi(), core.used.getOrDefault(real.imsi(), 0L) + 100);
        m.sendEthernet(eth(SERVER, m.mac(), 5));
        m.sendEthernet(eth(SERVER, m.mac(), 6));
        run(10, List.of(t), m);
        assertEquals(ModemLink.State.REFUSED, m.state(), m.message());
        assertTrue(m.message().contains("data limit"), m.message());
        assertEquals(0, t.link.deviceCount());
    }

    @Test
    void mmWaveOutOfReachIsNotHeard() {
        // 28 GHz far beyond its budget: 50 km in free space.
        Tower t = new Tower(CellTech.MMWAVE, "red", 1, 0, 20, 0);
        ModemLink m = modem(CellTech.MMWAVE, 2, 50_000, 2, 0);
        m.setSim(core.issue("red", 310_001_000_000_002L));
        run(60, List.of(t), m);
        assertNotEquals(ModemLink.State.CONNECTED, m.state());
        assertTrue(m.cells().isEmpty());
        // Near it, it attaches at a 256QAM rate.
        m.setPose(Pose.at("overworld", 100, 2, 0));
        run(40, List.of(t), m);
        assertEquals(ModemLink.State.CONNECTED, m.state(), m.message());
        assertTrue(m.downlinkCqi() >= 16, "cqi " + m.downlinkCqi());
    }

    @Test
    void handoverBetweenTowersOfOneCarrierKeepsTheMac() {
        Tower a = new Tower(CellTech.LTE, "red", 2, 0, 15, 0);
        Tower b = new Tower(CellTech.LTE, "red", 2, 400, 15, 0);
        Tower other = new Tower(CellTech.LTE, "blue", 2, 380, 15, 0);   // nearer, but another carrier's
        ModemLink m = modem(CellTech.LTE, 3, 20, 2, 0);
        m.setSim(core.issue("red", 310_001_000_000_003L));
        List<Tower> all = List.of(a, b, other);
        run(40, all, m);
        assertEquals(ModemLink.State.CONNECTED, m.state(), m.message());
        assertEquals(a.link.id(), m.serving().towerId());
        // Walk to the far tower, 10 m per tick, sending a frame every tick.
        for (int x = 20; x <= 390; x += 10) {
            m.setPose(Pose.at("overworld", x, 2, 0));
            m.sendEthernet(eth(SERVER, m.mac(), x / 10));
            run(1, all, m);
        }
        run(20, all, m);
        assertEquals(ModemLink.State.CONNECTED, m.state(), m.message() + " " + m.events());
        assertEquals(b.link.id(), m.serving().towerId(), "handed over to the near red tower: " + m.events());
        assertEquals(1, m.handovers(), m.events().toString());
        assertEquals(0, a.link.deviceCount(), "the old tower let go");
        assertEquals(1, b.link.deviceCount());
        assertEquals(0, other.link.attaches(), "never attached to the other carrier");
        // Frames kept flowing through the walk: across both towers, nearly every one reached a cable.
        int carried = a.cable.size() + b.cable.size();
        assertTrue(carried >= 35, "carried " + carried + " of 38");
        // Downlink now comes from the new tower.
        b.link.fromCable(eth(m.mac(), SERVER, 99));
        a.link.fromCable(eth(m.mac(), SERVER, 98));
        run(1, all, m);
        byte[] got = m.pollEthernet();
        assertNotNull(got);
        assertEquals(99, got[20]);
        assertNull(m.pollEthernet());
    }

    @Test
    void lostTowerFallsBackToSearch() {
        Tower t = new Tower(CellTech.LTE, "red", 1, 0, 15, 0);
        ModemLink m = modem(CellTech.LTE, 4, 50, 2, 0);
        m.setSim(core.issue("red", 310_001_000_000_004L));
        run(30, List.of(t), m);
        assertTrue(m.linkUp());
        t.link.setEnabled(false);
        run(30, List.of(t), m);
        assertFalse(m.linkUp());
        assertEquals(ModemLink.State.SEARCHING, m.state(), m.message());
    }
}
