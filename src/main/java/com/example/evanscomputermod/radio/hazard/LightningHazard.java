package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.amp.ExciterLink;
import com.example.evanscomputermod.radio.amp.TransmitChain;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.graph.AntennaGraph;
import com.example.evanscomputermod.radio.api.event.HazardEvent;
import net.minecraft.core.BlockPos;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.LightningBolt;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.event.entity.EntityJoinLevelEvent;

import java.util.ArrayList;
import java.util.List;

/**
 * Lightning on antennas (spec, Hazards: ungrounded tall antenna in a storm).
 * Where lightning lands is vanilla's business: it hits the highest block in
 * a column, and vanilla lightning rods (which join antennas as conductors,
 * {@code #evanscomputermod:rf_conductors}) pull strikes to themselves. When a
 * bolt lands within {@link #STRIKE_RADIUS} blocks of an antenna wired to a
 * transmit chain, the surge runs down the feedline towards the radio: every
 * device (tuner, amplifier, the exciter SDR) between the antenna and the first
 * {@code lightning_arrestor} is destroyed. With an arrestor right at the
 * antenna, nothing is. Needs {@code radioLightningDamage} and a hazard level
 * above off; each destruction posts a {@code HazardEvent} (LIGHTNING) and a
 * break event from the antenna builder's fake player.
 */
public final class LightningHazard {
    public static final double STRIKE_RADIUS = 3;

    private LightningHazard() {}

    static void onJoin(EntityJoinLevelEvent e) {
        if (e.getLevel() instanceof ServerLevel level && e.getEntity() instanceof LightningBolt bolt && !e.loadedFromDisk())
            strike(level, bolt.position());
    }

    /** A bolt landed at {@code at}: returns the devices destroyed. */
    public static List<BlockPos> strike(ServerLevel level, Vec3 at) {
        List<BlockPos> destroyed = new ArrayList<>();
        for (ExciterLink link : ExciterLink.in(level)) {
            TransmitChain chain = link.chain();
            Antenna antenna = link.antenna();
            if (chain.feed() == null || antenna == null || !hits(antenna, chain.feed(), at)) continue;
            if (!RadioGameRules.lightningDamage(level, chain.feed())) {
                link.noteHazard("lightning struck the antenna (lightning damage is off)");
                continue;
            }
            destroyed.addAll(surge(level, link, chain));
        }
        return destroyed;
    }

    /** Walks from the antenna down the feedline, destroying devices until an arrestor grounds the surge. */
    private static List<BlockPos> surge(ServerLevel level, ExciterLink link, TransmitChain chain) {
        List<BlockPos> out = new ArrayList<>();
        var hops = chain.hops();
        for (int j = hops.size() - 1; j >= 0; j--) {
            if (hops.get(j).line().arrestor()) {
                link.noteHazard("lightning grounded by the arrestor");
                return out;
            }
            BlockPos device = j == 0 ? chain.exciter() : hops.get(j - 1).pos();
            if (device == null) continue;
            String what = "lightning surge down the coax from the antenna at " + chain.feed().toShortString();
            if (HazardActions.destroy(level, device, link.owner(), HazardEvent.Kind.LIGHTNING, what,
                    new ItemStack(RadioHazardContent.MELTED_SCRAP.get())))
                out.add(device);
        }
        link.noteHazard(out.isEmpty() ? "lightning struck; nothing destroyed" : "lightning destroyed " + out.size() + " device(s): no arrestor");
        return out;
    }

    private static boolean hits(Antenna antenna, BlockPos feed, Vec3 at) {
        if (Vec3.atCenterOf(feed).distanceTo(at) <= STRIKE_RADIUS) return true;
        AntennaGraph g = antenna.graph();
        if (g == null) return false;
        for (AntennaGraph.Edge e : g.edges) if (distance(e.a(), e.b(), at) <= STRIKE_RADIUS) return true;
        return false;
    }

    private static double distance(AntennaGraph.Point a, AntennaGraph.Point b, Vec3 p) {
        double abx = b.x() - a.x(), aby = b.y() - a.y(), abz = b.z() - a.z();
        double len2 = abx * abx + aby * aby + abz * abz;
        double t = len2 == 0 ? 0 : ((p.x - a.x()) * abx + (p.y - a.y()) * aby + (p.z - a.z()) * abz) / len2;
        t = Math.max(0, Math.min(1, t));
        double x = a.x() + t * abx - p.x, y = a.y() + t * aby - p.y, z = a.z() + t * abz - p.z;
        return Math.sqrt(x * x + y * y + z * z);
    }
}
//?}
