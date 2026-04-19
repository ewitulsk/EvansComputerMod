package com.example.evanscomputermod.block;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.BlockGetter;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.Collections;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedList;
import java.util.List;
import java.util.Map;
import java.util.Queue;
import java.util.Set;

import org.jetbrains.annotations.Nullable;

/**
 * Rectangle validation + anchor selection for ScreenBlock clusters.
 * <p>
 * A valid cluster is a rectangular grid of {@link ScreenBlock}s that all
 * share the same {@link ScreenBlock#FACING} direction and at least one of
 * which is adjacent to a {@link TerminalBlock} on the opposite of its
 * display face. Discovery runs on the server thread only.
 */
public final class ScreenClusterDiscovery {

    private ScreenClusterDiscovery() {}

    /** Result of a successful cluster validation. */
    public record ClusterResult(
            BlockPos anchor,
            Direction facing,
            int cols,
            int rows,
            List<BlockPos> members
    ) {}

    /**
     * Called by the screen block on any placement / removal / neighbor change.
     * Walks to any adjacent terminal and triggers its rescan.
     */
    public static void triggerRescanNear(Level level, BlockPos screenPos) {
        if (level == null || level.isClientSide()) return;
        Set<BlockPos> seen = new HashSet<>();
        scheduleRescanFromScreen(level, screenPos, seen);
    }

    /**
     * Walk from this screen through connected same-facing screens looking
     * for a terminal adjacent to any of them. If found, schedule a rescan
     * on that terminal. Uses a small visited set to bound work.
     */
    private static void scheduleRescanFromScreen(Level level, BlockPos start, Set<BlockPos> seen) {
        BlockState startState = level.getBlockState(start);
        if (!(startState.getBlock() instanceof ScreenBlock)) {
            // Scan for neighboring terminals — handles the case of a removed
            // screen where `start` is no longer a screen block.
            for (Direction d : Direction.values()) {
                BlockPos t = start.relative(d);
                if (level.getBlockEntity(t) instanceof TerminalBlockEntity term) {
                    term.rescanScreenCluster();
                }
            }
            return;
        }
        Direction facing = startState.getValue(ScreenBlock.FACING);

        Queue<BlockPos> q = new LinkedList<>();
        q.add(start);
        seen.add(start);

        while (!q.isEmpty()) {
            BlockPos p = q.poll();
            for (Direction d : Direction.values()) {
                BlockPos n = p.relative(d);
                if (level.getBlockEntity(n) instanceof TerminalBlockEntity term) {
                    // Only if the terminal's opposite-of-display face matches
                    // the screen's facing (i.e. terminal "looks at" the screens
                    // from behind — same forward vector).
                    term.rescanScreenCluster();
                }
                if (seen.contains(n)) continue;
                BlockState ns = level.getBlockState(n);
                if (ns.getBlock() instanceof ScreenBlock && ns.getValue(ScreenBlock.FACING) == facing) {
                    seen.add(n);
                    q.add(n);
                }
            }
        }
    }

    /**
     * Validate a screen cluster rooted at a particular terminal.
     * Returns a result if one of the terminal's non-display-face neighbors is
     * a screen that forms a valid rectangle, else null.
     */
    @Nullable
    public static ClusterResult discover(Level level, BlockPos terminalPos, Direction terminalFacing) {
        if (level == null) return null;

        // Find adjacent screens — but the terminal's DISPLAY face is the
        // FACING direction. Screens should be on any other face and should
        // point in SOME direction. We try each neighbor.
        for (Direction d : Direction.values()) {
            if (d == terminalFacing) continue; // screen face: GUI, not screens
            BlockPos np = terminalPos.relative(d);
            BlockState ns = level.getBlockState(np);
            if (!(ns.getBlock() instanceof ScreenBlock)) continue;
            Direction screenFacing = ns.getValue(ScreenBlock.FACING);

            ClusterResult res = tryBuildRectangle(level, np, screenFacing, terminalPos);
            if (res != null) return res;
        }
        return null;
    }

    /**
     * BFS from a seed screen through all connected screens sharing the same
     * facing direction, then validate that they fill a perfect rectangle in
     * the facing plane and that one of them touches the owning terminal.
     */
    @Nullable
    private static ClusterResult tryBuildRectangle(BlockGetter level, BlockPos seed,
                                                    Direction facing, BlockPos terminalPos) {
        // Axes in the face plane. For any facing, we pick a "u" (horizontal)
        // and "v" (vertical) basis oriented so that when the viewer stands
        // in front of the screen (outside, on the normal side, looking INTO
        // the display face), u increases to the RIGHT and v increases UPWARD.
        //
        // Right-hand rule: right = forward × up, where forward = -normal
        // (viewer looks against the normal). E.g. NORTH-facing screen has
        // normal -z, viewer looks +z, so right = (+z)×(+y) = -x = WEST.
        Direction uAxisPos;
        Direction vAxisPos;
        switch (facing) {
            case NORTH -> { uAxisPos = Direction.WEST;  vAxisPos = Direction.UP; }
            case SOUTH -> { uAxisPos = Direction.EAST;  vAxisPos = Direction.UP; }
            case WEST  -> { uAxisPos = Direction.SOUTH; vAxisPos = Direction.UP; }
            case EAST  -> { uAxisPos = Direction.NORTH; vAxisPos = Direction.UP; }
            default -> { return null; } // Only horizontal facings supported
        }
        Direction uAxisNeg = uAxisPos.getOpposite();
        Direction vAxisNeg = vAxisPos.getOpposite();

        // BFS collect all connected same-facing screens in the 2D plane.
        Set<BlockPos> members = new HashSet<>();
        Queue<BlockPos> q = new LinkedList<>();
        members.add(seed);
        q.add(seed);

        while (!q.isEmpty()) {
            BlockPos p = q.poll();
            for (Direction d : new Direction[] { uAxisPos, uAxisNeg, vAxisPos, vAxisNeg }) {
                BlockPos n = p.relative(d);
                if (members.contains(n)) continue;
                BlockState ns = level.getBlockState(n);
                if (ns.getBlock() instanceof ScreenBlock &&
                        ns.getValue(ScreenBlock.FACING) == facing) {
                    members.add(n);
                    q.add(n);
                }
            }
        }

        // Compute 2D coordinates for each member in the (u, v) basis.
        // Project each BlockPos onto the u and v axes via dot-product with
        // their normal vectors (worldspace).
        Map<BlockPos, int[]> coords = new HashMap<>();
        int minU = Integer.MAX_VALUE, maxU = Integer.MIN_VALUE;
        int minV = Integer.MAX_VALUE, maxV = Integer.MIN_VALUE;
        for (BlockPos p : members) {
            int u = dot(p, uAxisPos);
            int v = dot(p, vAxisPos);
            coords.put(p, new int[] { u, v });
            if (u < minU) minU = u;
            if (u > maxU) maxU = u;
            if (v < minV) minV = v;
            if (v > maxV) maxV = v;
        }

        int cols = maxU - minU + 1;
        int rows = maxV - minV + 1;
        if (cols <= 0 || rows <= 0) return null;
        if (members.size() != cols * rows) return null;

        // Verify every (u,v) cell in the bounding box is filled.
        boolean[][] filled = new boolean[cols][rows];
        for (Map.Entry<BlockPos, int[]> e : coords.entrySet()) {
            int u = e.getValue()[0] - minU;
            int v = e.getValue()[1] - minV;
            filled[u][v] = true;
        }
        for (int u = 0; u < cols; u++) {
            for (int v = 0; v < rows; v++) {
                if (!filled[u][v]) return null;
            }
        }

        // At least one member must be adjacent to the owning terminal.
        boolean touchesTerminal = false;
        for (BlockPos p : members) {
            for (Direction d : Direction.values()) {
                if (p.relative(d).equals(terminalPos)) {
                    touchesTerminal = true;
                    break;
                }
            }
            if (touchesTerminal) break;
        }
        if (!touchesTerminal) return null;

        // Anchor = the member at (u=minU, v=maxV) — that's the top-left
        // corner when the cluster is viewed from outside (v increases upward
        // in the game world, and we want row 0 at the top of the screen,
        // so the anchor's v is maxV).
        BlockPos anchor = null;
        for (Map.Entry<BlockPos, int[]> e : coords.entrySet()) {
            if (e.getValue()[0] == minU && e.getValue()[1] == maxV) {
                anchor = e.getKey();
                break;
            }
        }
        if (anchor == null) return null;

        List<BlockPos> memberList = new ArrayList<>(members);
        Collections.sort(memberList, (a, b) -> {
            int ca = a.getX() * 31 + a.getY() * 17 + a.getZ();
            int cb = b.getX() * 31 + b.getY() * 17 + b.getZ();
            return Integer.compare(ca, cb);
        });

        return new ClusterResult(anchor, facing, cols, rows, memberList);
    }

    /** Dot product of a BlockPos with a unit direction vector. */
    private static int dot(BlockPos p, Direction d) {
        return p.getX() * d.getStepX() + p.getY() * d.getStepY() + p.getZ() * d.getStepZ();
    }
}
