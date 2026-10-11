package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.InterfaceBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.computer.CableRouter;
import com.example.evanscomputermod.computer.FiberChords;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.block.Mirror;
import net.minecraft.world.level.block.Rotation;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplate;

import java.util.*;

/**
 * The ISP router's own cables, routed knowing the building's rotation.
 *
 * <p>NIC numbering follows absolute directions (a terminal's non-screen faces in DOWN, UP,
 * NORTH, SOUTH, WEST, EAST order, then its Interface Block's faces), so which wall of a
 * rotated ISP faces eth2 or eth3 changes with the rotation, and the panels hang on
 * world-fixed sides of the mast. This computes, in the ISP template's coordinates:
 *
 * <ul>
 *   <li>eth2 (fiber to the previous village): from its face, through the room (preferring
 *       the ceiling layer), up the mast-side column under the "previous" panel;
 *   <li>eth3 (fiber to the next village): the same to the "next" panel;
 *   <li>eth1 (server LAN): along the ceiling, through the east wall at (15, 4, 10) and
 *       overhead across the yard to the Data Center's inlet cable at (21, 4, 10).
 * </ul>
 *
 * No two of these runs (nor any other port's face) touch, so each stays its own segment;
 * eth0's buried village cable runs under the floor.
 */
public final class IspCabling {
    private IspCabling() {}

    /** The interior box the runs may use (template coordinates). */
    public static final int X0 = 6, X1 = 14, Y0 = 1, Y1 = 4, Z0 = 6, Z1 = 14;
    public static final int MAST_X = 10, MAST_Z = 10;
    /** Through the east wall and across the yard, to the Data Center inlet at x = 21. */
    public static final BlockPos CONDUIT_GOAL = new BlockPos(14, 4, 10), DC_INLET = new BlockPos(21, 4, 10);

    /** World cells of the three runs, the cable arm masks, and the fiber ports' exits. */
    public record Result(Map<Long, Integer> cables, List<BlockPos> prevRun, List<BlockPos> nextRun, List<BlockPos> lanRun,
                         BlockPos prevExit, BlockPos nextExit, BlockPos lanExit) {}

    public static Rotation inverse(Rotation r) {
        return switch (r) {
            case CLOCKWISE_90 -> Rotation.COUNTERCLOCKWISE_90;
            case COUNTERCLOCKWISE_90 -> Rotation.CLOCKWISE_90;
            default -> r;
        };
    }

    /**
     * Local NIC exit cells in eth order for the terminal at {@code router} (template
     * coordinates) when the template is placed with {@code rotation}.
     */
    public static List<BlockPos> nicExits(TemplateScan scan, BlockPos router, Rotation rotation) {
        Rotation inv = inverse(rotation);
        Direction screen = scan.state(router.getX(), router.getY(), router.getZ()).getValue(TerminalBlock.FACING);
        List<BlockPos> exits = new ArrayList<>();
        List<Direction> interfaces = new ArrayList<>();
        for (Direction world : Direction.values()) {
            Direction local = inv.rotate(world);
            BlockPos n = router.relative(local);
            if (scan.state(n.getX(), n.getY(), n.getZ()).getBlock() instanceof InterfaceBlock) {
                interfaces.add(local);
                continue;
            }
            if (local != screen) exits.add(n);
        }
        Set<BlockPos> visited = new HashSet<>(List.of(router));
        for (Direction d : interfaces) visited.add(router.relative(d));
        for (Direction d : interfaces) {
            BlockPos block = router.relative(d);
            for (Direction world : Direction.values()) {
                BlockPos f = block.relative(inv.rotate(world));
                if (!visited.contains(f) && !(scan.state(f.getX(), f.getY(), f.getZ()).getBlock() instanceof InterfaceBlock))
                    exits.add(f);
            }
        }
        return exits;
    }

    private static long pack(BlockPos p) {
        return FiberChords.pack(p.getX(), p.getY(), p.getZ());
    }

    private static BlockPos unpack(long p) {
        return new BlockPos(FiberChords.unpackX(p), FiberChords.unpackY(p), FiberChords.unpackZ(p));
    }

    /**
     * Plan the runs. {@code prevSide}/{@code nextSide} are the world sides of the mast the
     * panels hang on; {@code withLan} routes eth1 to the Data Center. Null if the runs do
     * not fit (never for the generated templates; checked by a GameTest for every style,
     * rotation and side pair).
     */
    public static Result plan(TemplateScan scan, BlockPos origin, Rotation rotation, Direction prevSide, Direction nextSide,
                              boolean withLan) {
        BlockPos router = scan.findNbt("ecmRole", "isp.router");
        if (router == null) return null;
        Rotation inv = inverse(rotation);
        List<BlockPos> exits = nicExits(scan, router, rotation);
        if (exits.size() < 4) return null;
        Direction screen = scan.state(router.getX(), router.getY(), router.getZ()).getValue(TerminalBlock.FACING);
        int mastTop = scan.mastTop();
        Direction prevLocal = inv.rotate(prevSide), nextLocal = inv.rotate(nextSide);

        List<CableRouter.Run> runs = new ArrayList<>();
        runs.add(riser("eth2", exits.get(2), prevLocal, mastTop));
        runs.add(riser("eth3", exits.get(3), nextLocal, mastTop));
        if (withLan) {
            List<Long> conduit = new ArrayList<>();
            for (int x = CONDUIT_GOAL.getX() + 1; x < DC_INLET.getX(); x++) conduit.add(FiberChords.pack(x, 4, MAST_Z));
            runs.add(new CableRouter.Run("eth1", pack(exits.get(1)), pack(CONDUIT_GOAL), conduit));
        }
        Set<Long> forbidden = new HashSet<>();
        for (int i = 0; i < exits.size(); i++) if (i != 1 && i != 2 && i != 3) forbidden.add(pack(exits.get(i)));
        if (!withLan) forbidden.add(pack(exits.get(1)));
        forbidden.add(pack(router.relative(screen)));
        List<List<Long>> routed = CableRouter.routeAll(
                runs,
                p -> {
                    int x = FiberChords.unpackX(p), y = FiberChords.unpackY(p), z = FiberChords.unpackZ(p);
                    return x >= X0 && x <= X1 && y >= Y0 && y <= Y1 && z >= Z0 && z <= Z1 && scan.isAir(x, y, z);
                },
                forbidden,
                p -> FiberChords.unpackY(p) == Y1 ? 1 : 4);
        if (routed == null) return null;

        // To world space, with arm masks from the world neighbours.
        Map<Long, Integer> cables = new HashMap<>();
        List<List<BlockPos>> world = new ArrayList<>();
        for (int r = 0; r < routed.size(); r++) {
            List<BlockPos> cells = new ArrayList<>();
            for (long p : routed.get(r)) cells.add(toWorld(origin, rotation, unpack(p)));
            world.add(cells);
            BlockPos before = toWorld(origin, rotation, router);
            BlockPos after = r < 2 ? cells.get(cells.size() - 1).above()
                    : toWorld(origin, rotation, DC_INLET);
            for (int i = 0; i < cells.size(); i++) {
                BlockPos c = cells.get(i);
                int mask = 0;
                BlockPos prev = i == 0 ? before : cells.get(i - 1), next = i == cells.size() - 1 ? after : cells.get(i + 1);
                for (BlockPos n : new BlockPos[] {prev, next}) {
                    Direction d = Direction.fromDelta(n.getX() - c.getX(), n.getY() - c.getY(), n.getZ() - c.getZ());
                    if (d != null) mask |= 1 << d.ordinal();
                }
                cables.put(c.asLong(), mask);
            }
        }
        return new Result(cables, world.get(0), world.get(1), withLan ? world.get(2) : List.of(),
                world.get(0).get(0), world.get(1).get(0), withLan ? world.get(2).get(0) : null);
    }

    private static CableRouter.Run riser(String name, BlockPos exit, Direction side, int mastTop) {
        int x = MAST_X + side.getStepX(), z = MAST_Z + side.getStepZ();
        List<Long> column = new ArrayList<>();
        for (int y = Y1 + 1; y < mastTop; y++) column.add(FiberChords.pack(x, y, z));
        return new CableRouter.Run(name, pack(exit), FiberChords.pack(x, Y1, z), column);
    }

    public static BlockPos toWorld(BlockPos origin, Rotation rotation, BlockPos local) {
        return origin.offset(StructureTemplate.transform(local, Mirror.NONE, rotation, BlockPos.ZERO));
    }
}
//?}
