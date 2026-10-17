package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.InterfaceBlock;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.worldgen.TechVillageLocator;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.structure.BoundingBox;

import java.util.*;

/**
 * Reads a generated Tech Village and checks that its cables look the way they are wired: every
 * Network Cable and Interface Block in the ISP and the Data Center shows an arm toward each
 * neighbour it connects to (and none toward anything else), and the ISP router's Interface
 * Block and its cabled faces are joined to the router. Structure pieces place blocks without
 * recomputing their shapes, so an arm missing from a template or a planned run stays missing.
 * It only reads blocks.
 */
public final class TechCableChecks {
    private TechCableChecks() {}

    /** Null when every cable and interface block matches its neighbours, else what is wrong. */
    public static String check(ServerLevel level, TechVillageLocator.Visit visit, List<String> details) {
        BlockPos router = TechVillageLocator.role(level, visit, com.example.evanscomputermod.computer.WorldNetwork.ISP_ROUTER);
        if (router == null) return "no ISP router in the village";
        BlockState routerState = level.getBlockState(router);
        Direction screen = routerState.getValue(TerminalBlock.FACING);
        List<String> wrong = new ArrayList<>();
        int joined = 0;
        for (Direction d : Direction.values()) {
            if (d == screen) continue;
            BlockState n = level.getBlockState(router.relative(d));
            if (!(n.getBlock() instanceof NetworkCableBlock) && !(n.getBlock() instanceof InterfaceBlock)) continue;
            if (n.getValue(NetworkCableBlock.getPropertyForDirection(d.getOpposite()))) joined++;
            else wrong.add(name(n) + " on the router's " + d.getName() + " face has no arm to the router");
        }
        details.add("router " + router.toShortString() + " facing " + screen.getName() + ": " + joined + " faces joined");
        if (!(level.getBlockState(router.relative(screen.getCounterClockWise())).getBlock() instanceof InterfaceBlock)
                && !(level.getBlockState(router.relative(screen.getClockWise())).getBlock() instanceof InterfaceBlock))
            wrong.add("no Interface Block beside the router");

        int checked = 0;
        for (var piece : List.of(visit.isp(), visit.datacenter())) {
            if (piece == null) continue;
            BoundingBox b = piece.getBoundingBox();
            for (BlockPos p : BlockPos.betweenClosed(b.minX(), b.minY() - 3, b.minZ(), b.maxX(), b.maxY(), b.maxZ())) {
                BlockState s = level.getBlockState(p);
                if (!(s.getBlock() instanceof NetworkCableBlock) && !(s.getBlock() instanceof InterfaceBlock)) continue;
                checked++;
                for (Direction d : Direction.values()) {
                    BlockPos q = p.relative(d);
                    boolean expected = NetworkCableBlock.canConnectToFace(level.getBlockState(q), d.getOpposite(), level, q);
                    boolean shown = s.getValue(NetworkCableBlock.getPropertyForDirection(d));
                    if (expected != shown && wrong.size() < 20)
                        wrong.add(name(s) + " at " + p.toShortString() + (shown ? " shows an arm " : " has no arm ") + d.getName()
                                + " to " + name(level.getBlockState(q)));
                }
            }
        }
        details.add(checked + " cable and interface blocks checked");
        return wrong.isEmpty() ? null : String.join("; ", wrong);
    }

    private static String name(BlockState s) {
        return s.getBlock() instanceof InterfaceBlock ? "Interface Block"
                : s.getBlock() instanceof NetworkCableBlock ? "cable" : s.getBlock().getName().getString();
    }
}
//?}
