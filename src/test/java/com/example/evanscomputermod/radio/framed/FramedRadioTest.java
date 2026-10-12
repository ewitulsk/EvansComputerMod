package com.example.evanscomputermod.radio.framed;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.BasicRadioMedium;
import com.example.evanscomputermod.radio.phys.Materials;
import com.example.evanscomputermod.radio.cellular.CellAntennas;

import org.junit.jupiter.api.Test;

import java.nio.ByteBuffer;
import java.nio.ByteOrder;
import java.nio.charset.StandardCharsets;
import java.util.ArrayList;
import java.util.List;
import java.util.UUID;
import java.util.concurrent.atomic.AtomicLong;

/** The framed PHY radio ({@link FramedRadio}) on the reference medium, and the PHY profiles' channel plans, CQI table, link budgets and panel patterns. */
public class FramedRadioTest {

    final AtomicLong clock = new AtomicLong(10_000_000);
    /** Blocks of stone between x &lt; WALL_X and x &gt; WALL_X (one), in {@link #walled}. */
    static final double WALL_X = 10;
    boolean walls;
    final BasicRadioMedium medium = new BasicRadioMedium(clock::get, 7) {
        @Override
        protected double extraPathLossDb(RadioEndpoint tx, Pose a, RadioEndpoint rx, Pose b, double freqHz) {
            boolean crosses = walls && (a.x() - WALL_X) * (b.x() - WALL_X) < 0;
            return crosses ? Materials.STONE.lossDb(1, freqHz) : 0;
        }
    };
    final List<Radio> radios = new ArrayList<>();

    /** A test radio: an endpoint at (x, 64, z) and its framed radio. */
    final class Radio implements RadioEndpoint, FramedRadio.Host {
        final UUID id = UUID.randomUUID();
        final Pose pose;
        final FramedRadio r;
        final List<String> events = new ArrayList<>();

        Radio(double x, double z, PhyProfile p, FramedRadio.Role role, int address) {
            pose = Pose.at("o", x, 64, z);
            r = new FramedRadio(this, id);
            r.setProfile(p);
            r.setRole(role);
            r.setAddress(address);
            r.setActive(true);
            medium.register(this);
            radios.add(this);
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return r == null ? null : r.rxChannel(); }
        @Override public double noiseFigureDb() { return 5; }
        @Override public double sensitivityDbm() { return r.sensitivityDbm(); }
        @Override public double maxTxPowerDbm() { return 37; }
        @Override public void onReceive(Reception rec) { r.onReceive(rec); }
        @Override public RadioEndpoint endpoint() { return this; }
        @Override public RadioMedium medium() { return medium; }
        @Override public double maxTxDbm() { return 37; }
        @Override public void event(String name) { events.add(name); }

        void send(int dst, String text, int flags, long slot, int cqi, int sub) {
            r.write(FramedRecords.tx(flags, dst, slot, cqi, sub, 0, text.getBytes(StandardCharsets.UTF_8)), 0,
                    FramedRecords.TX_HEADER + text.getBytes(StandardCharsets.UTF_8).length);
        }

        void send(int dst, String text) {
            send(dst, text, 0, 0, 0, 0);
        }

        /** Every record waiting, decoded. */
        List<Rec> drain() {
            List<Rec> out = new ArrayList<>();
            byte[] all;
            while ((all = r.take(1 << 20)) != null) {
                for (int p = 0; p < all.length; ) {
                    int n = (all[p] & 0xff) | (all[p + 1] & 0xff) << 8;
                    out.add(new Rec(java.util.Arrays.copyOfRange(all, p, p + n)));
                    p += n;
                }
            }
            return out;
        }
    }

    /** A decoded RX or STATUS record. */
    static final class Rec {
        final byte[] b;
        final ByteBuffer bb;
        Rec(byte[] b) { this.b = b; bb = ByteBuffer.wrap(b).order(ByteOrder.LITTLE_ENDIAN); }
        int type() { return b[2] & 0xff; }
        boolean rx() { return type() == FramedRecords.RX; }
        int flags() { return b[3] & 0xff; }
        int src() { return bb.getInt(4); }
        int cqi() { return b[14] & 0xff; }
        int cqiSent() { return b[15] & 0xff; }
        double rssi() { return bb.getInt(16) / 10.0; }
        double sinr() { return bb.getInt(20) / 10.0; }
        long timestamp() { return bb.getLong(24); }
        long slot() { return bb.getLong(32); }
        int sub() { return b[40] & 0xff; }
        String text() { return new String(b, FramedRecords.RX_HEADER, b.length - FramedRecords.RX_HEADER, StandardCharsets.UTF_8); }
        int result() { return b[3] & 0xff; }
        int attempts() { return b[10] & 0xff; }
        long statusSlot() { return bb.getLong(12); }
    }

    /** Advance the clock {@code us} in 1 ms steps, pumping every radio. */
    void run(long us) {
        long end = clock.get() + us;
        while (clock.get() < end) {
            clock.addAndGet(Math.min(1000, end - clock.get()));
            for (Radio x : radios) x.r.pump();
        }
    }

    static List<Rec> rx(List<Rec> recs) {
        return recs.stream().filter(Rec::rx).toList();
    }

    @Test
    void framesAreDeliveredInRangeOnlyWithMeasurements() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio near = new Radio(100, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        Radio far = new Radio(100_000, 0, PhyProfile.LTE, FramedRadio.Role.UE, 3);
        Radio wrongChannel = new Radio(100, 5, PhyProfile.LTE, FramedRadio.Role.UE, 4);
        wrongChannel.r.setChannel(2);
        bs.r.setPower(0);
        bs.send(FramedRadio.BROADCAST, "hello");
        run(20_000);
        List<Rec> got = rx(near.drain());
        assertEquals(1, got.size());
        Rec f = got.get(0);
        assertEquals("hello", f.text());
        assertEquals(1, f.src());
        assertEquals(1, f.cqiSent(), "broadcasts go at CQI 1");
        // 0 dBm + 2 x 2.15 dBi - free space at 100 m, 791 MHz.
        double expect = 0 + 2 * 2.15 - BasicRadioMedium.freeSpaceLossDb(100, 791e6);
        assertEquals(expect, f.rssi(), 0.2);
        double noise = BasicRadioMedium.noiseDbm(10e6, 5);
        assertEquals(expect - noise, f.sinr(), 0.3);
        assertEquals(CqiTable.cqiFor(PhyProfile.LTE, f.sinr()), f.cqi());
        assertTrue(f.cqi() >= 14, "a strong LTE signal supports a high CQI, got " + f.cqi());
        assertEquals(0, rx(far.drain()).size(), "100 km at 0 dBm is out of range");
        assertEquals(0, rx(wrongChannel.drain()).size(), "a UE on channel 2 hears nothing on channel 1");
        assertTrue(near.events.contains("framed_rx"));
    }

    @Test
    void unicastFramesUseTheCqiThePeerReported() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(400, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        bs.r.setPower(-20);
        ue.r.setPower(-20);
        bs.send(FramedRadio.BROADCAST, "beacon-ish");
        run(20_000);
        Rec heard = rx(ue.drain()).get(0);
        int ueHearsBs = heard.cqi();
        assertTrue(ueHearsBs > 1 && ueHearsBs < 15, "a mid-range link, CQI " + ueHearsBs);
        ue.send(1, "report");   // carries the UE's measurement of the BS
        run(20_000);
        Rec up = rx(bs.drain()).get(0);
        assertEquals(1, up.cqiSent(), "the UE has no report from the BS yet");
        bs.send(2, "fast");
        run(20_000);
        Rec down = rx(ue.drain()).get(0);
        assertEquals("fast", down.text());
        assertEquals(ueHearsBs, down.cqiSent(), "the BS sends at the CQI the UE reported");
        assertEquals(up.cqi(), down.bb.get(41) & 0xff, "and reports how it hears the UE");
    }

    @Test
    void scheduledSlotsFollowTheAirtimeClock() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(50, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        long slot = bs.r.slot() + 40;
        bs.send(FramedRadio.BROADCAST, "at slot", 0, slot, 0, 0);
        run(30_000);
        assertEquals(0, rx(ue.drain()).size(), "not delivered before it has been on the air");
        run(20_000);
        List<Rec> got = rx(ue.drain());
        assertEquals(1, got.size());
        assertEquals(slot, got.get(0).slot());
        Rec status = bs.drain().get(0);
        assertEquals(FramedRecords.STATUS, status.type());
        assertEquals(FramedRecords.RESULT_SENT, status.result());
        assertEquals(slot, status.statusSlot());
        // A slot in the past is rejected.
        bs.send(FramedRadio.BROADCAST, "late", 0, bs.r.slot() - 2, 0, 0);
        assertEquals(FramedRecords.RESULT_REJECTED, bs.drain().get(0).result());
        // mmWave slots are 125 µs.
        Radio a = new Radio(0, 10, PhyProfile.MMWAVE, FramedRadio.Role.BS, 5);
        Radio b = new Radio(20, 10, PhyProfile.MMWAVE, FramedRadio.Role.UE, 6);
        long s2 = a.r.slot() + 100;
        a.send(FramedRadio.BROADCAST, "nr", 0, s2, 0, 0);
        run(30_000);
        Rec nr = rx(b.drain()).get(0);
        assertEquals(s2, nr.slot());
        assertTrue(nr.timestamp() > s2 * 125, "ends after its slot starts");
    }

    @Test
    void beaconOffloadRepeatsOnTheClock() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(50, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        byte[] rec = FramedRecords.beacon(0, 10, 0, 0, "cell 1".getBytes(StandardCharsets.UTF_8));
        bs.r.write(rec, 0, rec.length);
        run(1_000_000);
        List<Rec> got = rx(ue.drain());
        assertTrue(got.size() >= 99 && got.size() <= 101, "100 beacons in 1 s, got " + got.size());
        for (int i = 1; i < got.size(); i++) assertEquals(10_000, got.get(i).timestamp() - got.get(i - 1).timestamp(), "exactly 10 ms apart");
        assertTrue((got.get(0).flags() & FramedRecords.FLAG_BEACON) != 0);
        assertEquals("cell 1", got.get(0).text());
        assertTrue(bs.drain().isEmpty(), "beacons send no status records");
        byte[] stop = FramedRecords.beacon(0, 0, 0, 0, new byte[0]);
        bs.r.write(stop, 0, stop.length);
        run(200_000);   // the ones already on the air (up to the 150 ms lead) still arrive
        ue.drain();
        run(300_000);
        assertEquals(0, rx(ue.drain()).size(), "stopped");
    }

    @Test
    void lteGoesThroughAWallMmWaveDoesNot() {
        walls = true;
        Radio lteBs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio lteUe = new Radio(20, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        Radio mmBs = new Radio(0, 30, PhyProfile.MMWAVE, FramedRadio.Role.BS, 3);
        Radio mmUe = new Radio(20, 30, PhyProfile.MMWAVE, FramedRadio.Role.UE, 4);
        Radio mmClear = new Radio(8, 30, PhyProfile.MMWAVE, FramedRadio.Role.UE, 5);   // same side as the BS: control
        lteBs.send(FramedRadio.BROADCAST, "lte");
        mmBs.send(FramedRadio.BROADCAST, "mm");
        run(20_000);
        List<Rec> lte = rx(lteUe.drain());
        assertEquals(1, lte.size(), "LTE through one block of stone");
        double wall = Materials.STONE.lossDb(1, 791e6);
        assertTrue(wall > 20 && wall < 35, "a block of stone costs LTE " + wall + " dB");
        assertEquals(37 + 2 * 2.15 - BasicRadioMedium.freeSpaceLossDb(20, 791e6) - wall, lte.get(0).rssi(), 0.2,
                "the BS's 43 dBm default is capped at the 37 dBm hardware; the wall's loss is in the RSSI");
        assertEquals(0, rx(mmUe.drain()).size(), "28 GHz is stopped by one block of stone");
        assertEquals(1, rx(mmClear.drain()).size(), "and heard in line of sight");
    }

    @Test
    void aRadiosOwnFramesDoNotCollide() {
        // One radio's frames in one slot (data with a HARQ ACK going out, a beacon on the clock): its PHY
        // multiplexes them, as LTE multiplexes control, ACKs and data in one subframe, so none is lost.
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(30, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        long slot = bs.r.slot() + 5;
        bs.send(2, "first", 0, slot, 7, 0);
        bs.send(2, "second", 0, slot, 7, 0);
        ue.send(1, "up 1", FramedRecords.FLAG_HARQ, slot, 7, 1);
        ue.send(1, "up 2", 0, slot, 7, 2);
        run(30_000);
        assertEquals(List.of("first", "second"), rx(ue.drain()).stream().map(Rec::text).toList());
        assertEquals(List.of("up 1", "up 2"), rx(bs.drain()).stream().map(Rec::text).toList());
        assertEquals(0L, bs.r.status().get("rx_collisions"));
        assertEquals(0L, ue.r.status().get("rx_collisions"));
    }

    @Test
    void twoTransmittersInOneSlotCollide() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio a = new Radio(30, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        Radio b = new Radio(-30, 0, PhyProfile.LTE, FramedRadio.Role.UE, 3);
        long slot = bs.r.slot() + 5;
        a.send(1, "from a", 0, slot, 7, 0);
        b.send(1, "from b", 0, slot, 7, 0);
        run(30_000);
        List<Rec> got = rx(bs.drain());
        assertEquals(0, got.size(), "equal power in one slot at 16QAM: both lost, the first one too");
        // The second was dropped by the medium (the first was on the air when it was delivered); the first was
        // delivered clean and dropped by the radio's re-check when it ended.
        assertEquals(1L, bs.r.status().get("rx_collisions"));
        // Control: consecutive slots, or the same slot on different uplink sub-channels, both arrive.
        long s2 = bs.r.slot() + 5;
        a.send(1, "a again", 0, s2, 7, 0);
        b.send(1, "b again", 0, s2 + 10, 7, 0);
        long s3 = s2 + 30;
        a.send(1, "a sub 1", 0, s3, 7, 1);
        b.send(1, "b sub 3", 0, s3, 7, 3);
        run(80_000);
        List<String> texts = rx(bs.drain()).stream().map(Rec::text).toList();
        assertEquals(List.of("a again", "b again", "a sub 1", "b sub 3"), texts);
    }

    @Test
    void retuningFlushesFramesHeardOnTheOldChannel() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(50, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        bs.send(FramedRadio.BROADCAST, "old channel");
        run(20_000);
        bs.send(FramedRadio.BROADCAST, "in flight");
        ue.r.setChannel(2);   // one frame waiting in the queue, one still on the air
        run(20_000);
        assertEquals(0, rx(ue.drain()).size());
        ue.r.setChannel(1);
        bs.send(FramedRadio.BROADCAST, "back");
        run(20_000);
        assertEquals(List.of("back"), rx(ue.drain()).stream().map(Rec::text).toList());
    }

    @Test
    void harqAcknowledgesAndRetransmits() {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(50, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        ue.send(1, "ack me", FramedRecords.FLAG_HARQ, 0, 0, 0);
        run(50_000);
        assertEquals("ack me", rx(bs.drain()).get(0).text());
        Rec st = ue.drain().get(0);
        assertEquals(FramedRecords.STATUS, st.type());
        assertEquals(FramedRecords.RESULT_ACKED, st.result());
        assertEquals(1, st.attempts());
        // Nobody at address 9: four tries, then failed.
        ue.send(9, "nobody", FramedRecords.FLAG_HARQ, 0, 0, 0);
        run(100_000);
        Rec fail = ue.drain().get(0);
        assertEquals(FramedRecords.RESULT_FAILED, fail.result());
        assertEquals(1 + PhyProfile.HARQ_MAX_RETX, fail.attempts());
        assertEquals((long) PhyProfile.HARQ_MAX_RETX, ue.r.status().get("harq_retx"));
        assertEquals(0, rx(bs.drain()).size(), "frames for another address are not delivered");
    }

    @Test
    void deviceFileRoundTripAndErrors() throws Exception {
        Radio bs = new Radio(0, 0, PhyProfile.LTE, FramedRadio.Role.BS, 1);
        Radio ue = new Radio(50, 0, PhyProfile.LTE, FramedRadio.Role.UE, 2);
        FramedDeviceFd tx = new FramedDeviceFd(() -> bs.r), rxFd = new FramedDeviceFd(() -> ue.r);
        rxFd.setNonBlocking(true);
        assertEquals(com.example.evanscomputermod.computer.wasi.PipeFd.WOULD_BLOCK, rxFd.read(new byte[2048], 0, 2048));
        byte[] two = new byte[0];
        for (String s : List.of("one", "two")) {
            byte[] r = FramedRecords.tx(0, FramedRadio.BROADCAST, 0, 0, 0, 7, s.getBytes(StandardCharsets.UTF_8));
            byte[] c = java.util.Arrays.copyOf(two, two.length + r.length);
            System.arraycopy(r, 0, c, two.length, r.length);
            two = c;
        }
        assertEquals(two.length, tx.write(two, 0, two.length));
        run(20_000);
        assertTrue(rxFd.readReady());
        byte[] buf = new byte[2048];
        int n = rxFd.read(buf, 0, buf.length);
        assertEquals(2 * (FramedRecords.RX_HEADER + 3), n, "both records in one read");
        assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class, () -> tx.write(new byte[] {50, 0, 1}, 0, 3));
        bs.r.setActive(false);
        assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class, () -> tx.read(new byte[64], 0, 64));
        // Control lines.
        FramedDeviceFd.Ctl ctl = new FramedDeviceFd.Ctl(ue.r);
        byte[] cmd = "phy mmwave\nrole bs\nchannel 3\nphy_bw 50e6\npower 10\naddress 0x2a\n".getBytes(StandardCharsets.UTF_8);
        ctl.write(cmd, 0, cmd.length);
        assertEquals(PhyProfile.MMWAVE, ue.r.profile());
        assertEquals(FramedRadio.Role.BS, ue.r.role());
        assertEquals(3, ue.r.channel());
        assertEquals(50e6, ue.r.bandwidthHz());
        assertEquals(0x2a, ue.r.address());
        assertEquals(28.05e9 + 2 * 100e6, ue.r.txChannel().centerHz(), 1);
        byte[] text = new byte[4096];
        int len = ctl.read(text, 0, text.length);
        String s = new String(text, 0, len, StandardCharsets.UTF_8);
        assertTrue(s.contains("phy mmwave\n") && s.contains("address 0000002a\n") && s.contains("power_dbm 10.0\n"), s);
        byte[] bad = "channel 9\n".getBytes(StandardCharsets.UTF_8);
        assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class, () -> ctl.write(bad, 0, bad.length));
    }

    // ------------------------------------------------------------ PHY profiles (moved from the v1 CellularTest)

    @Test
    void channelPlansSitInsideTheirBandsAndAvoidTheGaps() {
        for (int n = 1; n <= PhyProfile.LTE.channels; n++) {
            for (Channel c : List.of(PhyProfile.LTE.downlink(n), PhyProfile.LTE.uplink(n))) {
                assertEquals(Band.UHF, c.band(), "LTE ch " + n);
                assertTrue(c.lowHz() >= 690e6 && c.highHz() <= 900e6, "LTE ch " + n + " at " + c.centerHz());
            }
        }
        for (int n = 1; n <= PhyProfile.MMWAVE.channels; n++) {
            for (Channel c : List.of(PhyProfile.MMWAVE.downlink(n), PhyProfile.MMWAVE.uplink(n))) {
                assertEquals(Band.MICROWAVE, c.band(), "mmWave ch " + n);
                assertTrue(c.centerHz() > 27e9 && c.centerHz() < 29e9);
            }
        }
        for (PhyProfile t : PhyProfile.values()) {
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
        for (int c = 2; c <= CqiTable.MAX_CQI; c++) {
            assertTrue(CqiTable.efficiency(c) > CqiTable.efficiency(c - 1));
            assertTrue(CqiTable.requiredSinrDb(c) > CqiTable.requiredSinrDb(c - 1));
        }
        // LTE 10 MHz at 64QAM CQI 15: 5.5547 b/s/Hz * 10 MHz * 0.75 = 41.7 Mbit/s.
        assertEquals(41.66e6, CqiTable.downlinkRateBps(PhyProfile.LTE, 15), 0.05e6);
        // NR 100 MHz at 256QAM CQI 18: 7.4063 * 100 MHz * 0.82 = 607 Mbit/s.
        assertEquals(607.3e6, CqiTable.downlinkRateBps(PhyProfile.MMWAVE, 18), 0.5e6);
        // LTE stops at CQI 15 however good the signal; mmWave goes to 18.
        assertEquals(15, CqiTable.cqiFor(PhyProfile.LTE, 60));
        assertEquals(18, CqiTable.cqiFor(PhyProfile.MMWAVE, 60));
        // Link adaptation keeps a 3 dB margin: 64QAM CQI 10 needs 11.7 dB, so 14.6 dB is not yet enough.
        assertEquals(9, CqiTable.cqiFor(PhyProfile.LTE, 14.6));
        assertEquals(10, CqiTable.cqiFor(PhyProfile.LTE, 14.7));
        assertEquals(CqiTable.ROBUST_CQI, CqiTable.cqiFor(PhyProfile.LTE, -20));
        assertEquals(CqiTable.ROBUST_CQI, CqiTable.cqiFor(PhyProfile.LTE, Double.NaN));
        // The medium sees the same thresholds through the modulation name.
        assertEquals(5.9, BasicRadioMedium.requiredSinrDb(CqiTable.modulation(7)), 1e-9);
        assertEquals(-6.7, BasicRadioMedium.requiredSinrDb("CELL-CQI1"), 1e-9);
        // A 1500-byte frame at LTE CQI 15 is 288 us plus a 71 us symbol.
        assertEquals(71 + 288, CqiTable.airtimeUs(PhyProfile.LTE, 1500, CqiTable.downlinkRateBps(PhyProfile.LTE, 15)), 1);
    }

    @Test
    void linkBudgets() {
        // Noise in 10 MHz with a 7 dB NF: -174 + 70 + 7 = -97 dBm; sensitivity at the robust CQI 7.7 dB under it.
        assertEquals(-97, CqiTable.noiseDbm(10e6, 7), 1e-9);
        assertEquals(-104.7, CqiTable.ueSensitivityDbm(PhyProfile.LTE), 1e-9);
        // LTE downlink 300 m in free space: 43 dBm + 14 dBi + 2.15 dBi - 80.1 dB = -21 dBm: top CQI.
        double fs = BasicRadioMedium.freeSpaceLossDb(300, PhyProfile.LTE.downlink(1).centerHz());
        assertEquals(80.0, fs, 0.5);
        double rx = PhyProfile.LTE.towerTxDbm + PhyProfile.LTE.towerPanelGainDbi + 2.15 - fs;
        double sinr = rx - CqiTable.noiseDbm(PhyProfile.LTE.bandwidthHz, PhyProfile.LTE.ueNoiseFigureDb);
        assertEquals(15, CqiTable.cqiFor(PhyProfile.LTE, sinr));
        // Reciprocity: the uplink estimate from that beacon also clears 64QAM on a 2.5 MHz slice.
        double ul = CqiTable.uplinkSinrEstimateDb(PhyProfile.LTE, rx - PhyProfile.LTE.towerPanelGainDbi * 0, PhyProfile.LTE.towerTxDbm);
        assertTrue(ul > 30, "uplink SINR " + ul);
        // mmWave free space at 28 GHz is ~31 dB worse than LTE at 800 MHz for the same distance.
        double mm = BasicRadioMedium.freeSpaceLossDb(300, PhyProfile.MMWAVE.downlink(1).centerHz());
        assertEquals(31, mm - fs, 0.5);
    }

    @Test
    void antennaPatterns() {
        var lte = CellAntennas.tower(CellAntennas.panelFor(PhyProfile.LTE), List.of(new double[] {0, 1}), 0);   // facing south (+Z)
        assertEquals(14 - 12 * Math.pow(3 / 20.0, 2), lte.gainDbi(0, 0, 1), 0.01);       // boresight, 3 deg below the tilt
        assertTrue(lte.gainDbi(0, 0, 1) - lte.gainDbi(1, 0, 0) > 10, "65 degree sector");
        assertEquals(14 - 25, lte.gainDbi(0, 0, -1), 0.01);                               // front-to-back
        var four = CellAntennas.tower(CellAntennas.panelFor(PhyProfile.LTE),
                List.of(new double[] {0, 1}, new double[] {1, 0}, new double[] {0, -1}, new double[] {-1, 0}), 0);
        assertTrue(four.gainDbi(0, 0, -1) > 13, "four panels cover all round");
        var mm = CellAntennas.tower(CellAntennas.panelFor(PhyProfile.MMWAVE), List.of(new double[] {1, 0}), 0);
        assertEquals(20, mm.gainDbi(1, 0, 0), 0.01);
        assertTrue(mm.gainDbi(Math.cos(Math.toRadians(50)), 0, Math.sin(Math.toRadians(50))) > 16, "steered 50 degrees off");
        assertEquals(-5, mm.gainDbi(-1, 0, 0), 0.01);
        var none = CellAntennas.tower(CellAntennas.panelFor(PhyProfile.LTE), List.of(), 0);
        assertTrue(none.peakGainDbi() < -50);
    }
}
