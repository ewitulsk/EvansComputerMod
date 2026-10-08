package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.AntennaPattern;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Emission;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.api.RadioEndpoint;
import com.example.evanscomputermod.radio.api.RadioMedium;
import com.example.evanscomputermod.radio.api.Reception;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.testing.scenario.RadioScenarios;
import net.minecraft.core.BlockPos;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.List;
import java.util.UUID;
import java.util.concurrent.CopyOnWriteArrayList;

/**
 * Radio &amp; Wireless, namespace {@code ecm_radio} (1.21.1). Scenario-backed
 * tests run the same definitions as {@code /ecm scenario spawn} through
 * {@link TestDriver#scenario}; this class also holds direct medium checks.
 */
@GameTestHolder(RadioTests.NS)
@PrefixGameTestTemplate(false)
public final class RadioTests {
    static final String NS = "ecm_radio";
    static final String STRUCTURE = "gametest_radio";

    /** The server's medium is running and carries a frame between two endpoints; a third out of range hears nothing (control). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".medium")
    public static void medium_delivers_frame_in_range_only(GameTestHelper h) {
        RadioMedium medium = RadioMediumHooks.medium();
        String dim = h.getLevel().dimension().location().toString();
        BlockPos o = h.absolutePos(BlockPos.ZERO);
        Channel ch = Channel.wifi24(6);
        TestEndpoint tx = new TestEndpoint(Pose.at(dim, o.getX(), o.getY() + 2, o.getZ()), ch);
        TestEndpoint near = new TestEndpoint(Pose.at(dim, o.getX() + 20, o.getY() + 2, o.getZ()), ch);
        TestEndpoint far = new TestEndpoint(Pose.at(dim, o.getX() + 200_000, o.getY() + 2, o.getZ()), ch);
        String[] failure = {null};
        if (medium == null) failure[0] = "no radio medium running";
        else {
            medium.register(tx);
            medium.register(near);
            medium.register(far);
            medium.transmit(tx, Emission.frame(ch, 20, medium.nowMicros(), 200, "DSSS-1", 1e6, new byte[] {1, 2, 3}));
            medium.unregister(tx);
            medium.unregister(near);
            medium.unregister(far);
        }
        TestDriver.drive(h, NS, "medium_delivers_frame_in_range_only", () -> {
            if (near.got.size() != 1) {
                failure[0] = "near receiver got " + near.got.size() + " frames";
                return false;
            }
            if (!far.got.isEmpty()) {
                failure[0] = "far receiver heard the frame";
                return false;
            }
            return true;
        }, () -> failure[0]);
    }

    /** Every radio scenario as a test. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".scenarios")
    public static void scenarios_registered(GameTestHelper h) {
        TestDriver.drive(h, NS, "scenarios_registered", () -> RadioScenarios.ALL != null, () -> null);
    }

    /** dhcpd on one computer leases to dhclient on another over a cable (with a no-server control first). */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".dhcp_lan")
    public static void dhcp_lan(GameTestHelper h) {
        TestDriver.scenario(h, NS, RadioScenarios.ALL.get("dhcp_lan"));
    }

    static final class TestEndpoint implements RadioEndpoint {
        final UUID id = UUID.randomUUID();
        final Pose pose;
        final Channel channel;
        final List<Reception> got = new CopyOnWriteArrayList<>();

        TestEndpoint(Pose pose, Channel channel) {
            this.pose = pose;
            this.channel = channel;
        }

        @Override public UUID id() { return id; }
        @Override public Pose pose() { return pose; }
        @Override public AntennaPattern antenna() { return AntennaPattern.VERTICAL_DIPOLE; }
        @Override public Channel tunedChannel() { return channel; }
        @Override public double maxTxPowerDbm() { return 20; }
        @Override public void onReceive(Reception r) { got.add(r); }
    }

    private RadioTests() {}
}
//?}
