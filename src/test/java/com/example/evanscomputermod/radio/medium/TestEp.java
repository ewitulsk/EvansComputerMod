package com.example.evanscomputermod.radio.medium;

import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.Reception;

import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;
import java.util.concurrent.atomic.AtomicLong;

/** Test endpoint: settable pose/channel/antenna, records receptions and how often the medium asked if it listens. */
final class TestEp implements RadioEndpoint {
    final UUID id = UUID.randomUUID();
    volatile Pose pose;
    volatile Channel ch;
    volatile AntennaPattern antenna = AntennaPattern.VERTICAL_DIPOLE;
    volatile double speed;
    final List<Reception> got = new CopyOnWriteArrayList<>();
    final AtomicLong asked = new AtomicLong();
    final boolean record;

    TestEp(double x, double y, double z, Channel ch) {
        this(x, y, z, ch, true);
    }

    TestEp(double x, double y, double z, Channel ch, boolean record) {
        this.pose = Pose.at("minecraft:overworld", x, y, z);
        this.ch = ch;
        this.record = record;
    }

    @Override public UUID id() { return id; }
    @Override public Pose pose() { return pose; }
    @Override public AntennaPattern antenna() { return antenna; }
    @Override public Channel tunedChannel() { return ch; }
    @Override public double maxTxPowerDbm() { return 20; }
    @Override public double speedMps() { return speed; }
    @Override public boolean listening() { asked.incrementAndGet(); return ch != null; }
    @Override public void onReceive(Reception r) { if (record) got.add(r); }
}
