package com.example.evanscomputermod.radio.medium;

import static org.junit.jupiter.api.Assertions.*;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Band;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.PassiveEndpoint;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.phys.NearField;

import org.junit.jupiter.api.Test;

import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;

/** Passive (energise-to-answer) endpoints on the near-field shard of {@link WorldRadioMedium}. */
public class PassiveEndpointTest {

    static final Channel NF = new Channel(13.56e6, 200e3);
    static final double READER_DBM = 10;

    final AtomicLong clock = new AtomicLong(1_000_000);
    final GridWorld world = new GridWorld(64, RfBlock.DIRT);
    final WorldRadioMedium.WorldAccess access = new WorldRadioMedium.WorldAccess() {
        @Override public RfWorld world(String d) { return world; }
        @Override public WorldRadioMedium.Conditions conditions(String d) { return WorldRadioMedium.Conditions.DEFAULT; }
    };
    final WorldRadioMedium m = new WorldRadioMedium(clock::get, 7);
    long tick = 100;

    /** A tag: isotropic coil, wakes at −45 dBm, remembers the field that powers it. */
    final class Tag implements PassiveEndpoint {
        final UUID id = UUID.randomUUID();
        volatile Pose pose;
        volatile double fieldDbm = Double.NEGATIVE_INFINITY;
        volatile long until = Long.MIN_VALUE;
        final List<Reception> got = new CopyOnWriteArrayList<>();

        Tag(double x, double y, double z) {
            pose = Pose.at("minecraft:overworld", x, y, z);
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.ISOTROPIC; }
        @Override public Channel tunedChannel() { return NF; }
        @Override public double wakeThresholdDbm() { return -45; }
        @Override public boolean energised() { return clock.get() < until; }
        @Override public double maxTxPowerDbm() { return energised() ? fieldDbm - backscatterLossDb() : Double.NEGATIVE_INFINITY; }
        @Override public void energise(double fieldPowerDbm, long untilMicros) { fieldDbm = fieldPowerDbm; until = untilMicros; }
        @Override public void onReceive(Reception r) { got.add(r); }
    }

    void tick() {
        m.tick(tick++, access);
    }

    TestEp reader(double x) {
        TestEp r = new TestEp(x, 65.5, 0.5, NF);
        r.antenna = AntennaPattern.ISOTROPIC;
        return r;
    }

    Emission field(long micros) {
        return Emission.energy(NF, READER_DBM, clock.get(), micros);
    }

    Emission reply() {
        return Emission.frame(NF, 20, clock.get(), 500, "NF-106", 106e3, new byte[] {0x50, 0x41, 0x53, 0x53});
    }

    @Test
    void nearFieldIsACarveOutInsideHf() {
        assertEquals(Band.NEAR_FIELD, Band.of(13.56e6));
        assertEquals(Band.NEAR_FIELD, NF.band());
        assertEquals(Band.HF, Band.of(14e6));
        assertEquals(Band.HF, Band.of(13.5e6));
    }

    @Test
    void fieldWakesTheTagAtOneMetreNotAtFour() {
        TestEp near = reader(0.5), far = reader(5.5);
        Tag tag = new Tag(1.5, 65.5, 0.5);
        for (var e : List.of(near, far)) m.register(e);
        m.register(tag);
        tick();
        tick();
        assertNotNull(m.link(near, tag, NF.centerHz()), "traced");
        m.transmit(far, field(1_000));
        assertFalse(tag.energised(), "4 m away: the field is " + (READER_DBM - NearField.lossDb(4, NF.centerHz())) + " dBm");
        assertEquals(0, near.got.size() + far.got.size(), "energy is never delivered as a frame");
        m.transmit(near, field(1_000));
        assertTrue(tag.energised());
        // Isotropic, no fading, no excess: only the generic spectral mask's in-band weight (−0.15 dB) is left.
        assertEquals(READER_DBM - NearField.lossDb(1, NF.centerHz()), tag.fieldDbm, 0.3, "isotropic, no fading, no excess");
        assertEquals(clock.get() + 1_000 + WorldRadioMedium.ENERGY_HOLD_MICROS, tag.until, "held two ticks past the field");
        clock.addAndGet(1_000 + WorldRadioMedium.ENERGY_HOLD_MICROS);
        assertFalse(tag.energised(), "cold again once the hold expires");
    }

    @Test
    void coldTagIsDroppedEnergisedTagBackscattersWithinTheField() {
        TestEp near = reader(0.5), far = reader(5.5);
        Tag tag = new Tag(1.5, 65.5, 0.5);
        for (var e : List.of(near, far)) m.register(e);
        m.register(tag);
        tick();
        tick();
        m.transmit(tag, reply());
        assertEquals(1, m.droppedColdTransmits(), "a cold tag has no power to answer with");
        assertEquals(0, near.got.size());
        assertEquals(Double.NEGATIVE_INFINITY, tag.maxTxPowerDbm());

        m.transmit(near, field(2_000));
        assertTrue(tag.energised());
        clock.addAndGet(2_500);   // the field is over, the hold keeps the tag up
        m.transmit(tag, reply());
        assertEquals(1, m.droppedColdTransmits());
        assertEquals(1, near.got.size(), "the reader hears the reply at 1 m");
        Reception r = near.got.get(0);
        assertEquals(tag.fieldDbm - tag.backscatterLossDb(), r.emission().powerDbm(), 1e-9, "clamped to field − 20 dB, not the 20 dBm asked");
        assertEquals(r.emission().powerDbm() - NearField.lossDb(1, NF.centerHz()), r.rssiDbm(), 0.3);
        assertTrue(r.rssiDbm() >= -90, "within the spec's reader sensitivity: " + r.rssiDbm());
        assertEquals(0, far.got.size(), "the reply at 4 m is " + (r.emission().powerDbm() - NearField.lossDb(4, NF.centerHz())) + " dBm");
    }

    @Test
    void ironBlockBetweenReaderAndTagShields() {
        TestEp reader = reader(0.5);
        Tag open = new Tag(2.5, 65.5, 0.5), shielded = new Tag(0.5, 65.5, 2.5);
        world.set(0, 65, 1, RfBlock.IRON);
        m.register(reader);
        m.register(open);
        m.register(shielded);
        tick();
        tick();
        assertNotNull(m.link(reader, shielded, NF.centerHz()));
        m.transmit(reader, field(1_000));
        assertTrue(open.energised(), "2 m of air: " + (READER_DBM - NearField.lossDb(2, NF.centerHz())) + " dBm");
        assertFalse(shielded.energised(), "2 m through an iron block");
        assertTrue(m.pathGainDb(reader, shielded, NF.centerHz()) < m.pathGainDb(reader, open, NF.centerHz()) - 60);
    }

    @Test
    void nf106HasAPerModel() {
        assertTrue(PerModels.per("NF-106", 30, 64, 200e3, 106e3) < 0.01);
        assertTrue(PerModels.per("NF-106", -20, 64, 200e3, 106e3) > 0.5);
        assertTrue(PerModels.per("NF-106", 15, 64, 200e3, 0) < PerModels.per("NF-106", 5, 64, 200e3, 0), "nominal rate when none given");
    }
}
