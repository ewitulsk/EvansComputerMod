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

    /**
     * A bridge port joins a segment, forwards a wireless client's frame onto the
     * cable with the client's MAC, learns that MAC, and receives unicast to it;
     * a NIC on another segment hears nothing (control).
     */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".bridge")
    public static void bridge_port_learns_client_macs(GameTestHelper h) {
        var hub = com.example.evanscomputermod.computer.NetworkHub.getInstance();
        var cables = com.example.evanscomputermod.computer.CableNetworkManager.getInstance();
        byte[] wired = {0x02, 0x7a, 1, 1, 1, 1}, port = {0x02, 0x7b, 2, 2, 2, 2},
                client = {0x02, 0x7c, 3, 3, 3, 3}, other = {0x02, 0x7d, 4, 4, 4, 4};
        List<byte[]> portGot = new CopyOnWriteArrayList<>();
        hub.registerNic(wired, (irq, p) -> {});
        hub.registerNic(other, (irq, p) -> {});
        hub.registerBridgePort(port, portGot::add);
        String link = "radio-test-bridge-" + UUID.randomUUID();
        cables.logicalLink(link, wired, port, true);
        String[] failure = {null};
        try {
            byte[] up = ethernet(new byte[] {-1, -1, -1, -1, -1, -1}, client);
            hub.transmitFromPort(port, up);
            byte[] atWired = hub.receive(wired);
            if (atWired == null || !java.util.Arrays.equals(java.util.Arrays.copyOfRange(atWired, 6, 12), client))
                failure[0] = "wired NIC didn't get the client's broadcast with the client's source MAC";
            else if (!java.util.Arrays.equals(hub.bridgePortOf(client), port))
                failure[0] = "client MAC not learned behind the port";
            else if (hub.receive(other) != null)
                failure[0] = "control: a NIC on no shared segment heard the frame";
            else {
                hub.transmit(wired, ethernet(client, wired));
                if (portGot.size() != 1 || !java.util.Arrays.equals(java.util.Arrays.copyOfRange(portGot.get(0), 0, 6), client))
                    failure[0] = "unicast to the client didn't reach the bridge port: " + portGot.size();
                hub.forgetBridged(client);
                if (failure[0] == null && hub.bridgePortOf(client) != null) failure[0] = "forgetBridged kept the entry";
            }
        } finally {
            cables.removeLogicalLink(link);
            hub.unregisterBridgePort(port);
            hub.unregisterNic(wired);
            hub.unregisterNic(other);
        }
        TestDriver.drive(h, NS, "bridge_port_learns_client_macs", () -> failure[0] == null, () -> failure[0]);
    }

    private static byte[] ethernet(byte[] dst, byte[] src) {
        byte[] f = new byte[60];
        System.arraycopy(dst, 0, f, 0, 6);
        System.arraycopy(src, 0, f, 6, 6);
        f[12] = 0x08;   // IPv4 EtherType; payload zeros
        return f;
    }

    /** Every radio scenario as a test. */
    @GameTest(template = STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".scenarios")
    public static void scenarios_registered(GameTestHelper h) {
        TestDriver.drive(h, NS, "scenarios_registered", () -> RadioScenarios.ALL != null, () -> null);
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
