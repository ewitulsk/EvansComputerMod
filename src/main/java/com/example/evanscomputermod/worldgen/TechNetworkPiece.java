package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.computer.WorldNetwork;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.nbt.*;
import net.minecraft.util.RandomSource;
import net.minecraft.world.level.ChunkPos;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.WorldGenLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.structure.BoundingBox;
import net.minecraft.world.level.levelgen.structure.StructurePiece;
import net.minecraft.world.level.levelgen.structure.TerrainAdjustment;
import net.minecraft.world.level.levelgen.structure.pieces.StructurePieceSerializationContext;

import java.util.*;

/**
 * A village's physical computer network, appended to the jigsaw village after it
 * expands: the house router/PC pairs and the buried access cable joining every house
 * router's DOWN face (eth0, WAN) to the ISP router's DOWN face (eth0, access LAN); the
 * two Fiber Patch Panels beside the mast top and the ISP router's own cables (eth2 and
 * eth3 up the mast to them, eth1 overhead to the Data Center, see {@link IspCabling}).
 * When it places the chunk holding the router it records where the fiber ports and
 * panels are, so the ring's links can follow the physical cabling.
 *
 * <p>The cable is a graph of (x, z) cells. Cells under a rigid building have a fixed
 * height (one block below its floor); street cells resolve at placement time to one
 * block under the road surface, read from the same {@code WORLD_SURFACE_WG} heightmap
 * the terrain-matching streets use. Between neighbouring cells of different heights the
 * vertical riser is placed in the higher cell's column (always underground), so the
 * cable stays face-connected while each chunk places only its own blocks. Rigid-free
 * pieces never get beards: this piece declares no terrain adjustment.
 */
public final class TechNetworkPiece extends StructurePiece
        implements net.neoforged.neoforge.common.world.PieceBeardifierModifier {
    public static final int SURFACE = Integer.MIN_VALUE;

    /** Cable cell: x, z, fixed y (or SURFACE), and the top of a drop to a terminal (or SURFACE). */
    public record Node(int x, int z, int fixedY, int dropTop) {}

    /** A provisioned terminal: role, position and screen facing. */
    public record Terminal(String role, BlockPos pos, Direction facing) {}

    /** A Fiber Patch Panel and the side it faces (away from the mast). */
    public record Panel(BlockPos pos, Direction facing) {}

    private final int village;
    private final List<Node> nodes;
    private final int[][] adjacency;
    private final List<Terminal> terminals;
    /**
     * Fixed cables, packed pos -> direction mask: over each house router/PC pair (LAN) and
     * the ISP router's runs (fiber ports up the mast, server LAN to the Data Center).
     */
    private final Map<Long, Integer> lanCables;
    private final List<Panel> panels;
    /** {prev port exit, prev panel, next port exit, next panel} (packed), or empty. */
    private final long[] fiberEnds;

    public TechNetworkPiece(int village, List<Node> nodes, int[][] adjacency, List<Terminal> terminals,
                            Map<Long, Integer> lanCables, List<Panel> panels, long[] fiberEnds, BoundingBox box) {
        super(TechWorldgen.NETWORK_PIECE.get(), 0, box);
        this.village = village;
        this.nodes = nodes;
        this.adjacency = adjacency;
        this.terminals = terminals;
        this.lanCables = lanCables;
        this.panels = panels;
        this.fiberEnds = fiberEnds;
    }

    public TechNetworkPiece(CompoundTag tag) {
        super(TechWorldgen.NETWORK_PIECE.get(), tag);
        village = tag.getInt("village");
        int[] n = tag.getIntArray("nodes");
        nodes = new ArrayList<>();
        for (int i = 0; i + 3 < n.length; i += 4) nodes.add(new Node(n[i], n[i + 1], n[i + 2], n[i + 3]));
        ListTag adj = tag.getList("adjacency", Tag.TAG_INT_ARRAY);
        adjacency = new int[nodes.size()][];
        for (int i = 0; i < nodes.size(); i++) adjacency[i] = i < adj.size() ? adj.getIntArray(i) : new int[0];
        terminals = new ArrayList<>();
        for (Tag t : tag.getList("terminals", Tag.TAG_COMPOUND)) {
            CompoundTag c = (CompoundTag) t;
            terminals.add(new Terminal(c.getString("role"), BlockPos.of(c.getLong("pos")),
                    Direction.from3DDataValue(c.getInt("facing"))));
        }
        lanCables = new HashMap<>();
        CompoundTag lan = tag.getCompound("lan");
        for (String k : lan.getAllKeys()) lanCables.put(Long.parseLong(k), lan.getInt(k));
        panels = new ArrayList<>();
        for (Tag t : tag.getList("panels", Tag.TAG_COMPOUND)) {
            CompoundTag c = (CompoundTag) t;
            panels.add(new Panel(BlockPos.of(c.getLong("pos")), Direction.from3DDataValue(c.getInt("facing"))));
        }
        fiberEnds = tag.getLongArray("fiberEnds");
    }

    @Override
    protected void addAdditionalSaveData(StructurePieceSerializationContext context, CompoundTag tag) {
        tag.putInt("village", village);
        int[] n = new int[nodes.size() * 4];
        for (int i = 0; i < nodes.size(); i++) {
            Node node = nodes.get(i);
            n[i * 4] = node.x;
            n[i * 4 + 1] = node.z;
            n[i * 4 + 2] = node.fixedY;
            n[i * 4 + 3] = node.dropTop;
        }
        tag.putIntArray("nodes", n);
        ListTag adj = new ListTag();
        for (int[] a : adjacency) adj.add(new IntArrayTag(a));
        tag.put("adjacency", adj);
        ListTag list = new ListTag();
        for (Terminal t : terminals) {
            CompoundTag c = new CompoundTag();
            c.putString("role", t.role);
            c.putLong("pos", t.pos.asLong());
            c.putInt("facing", t.facing.get3DDataValue());
            list.add(c);
        }
        tag.put("terminals", list);
        CompoundTag lan = new CompoundTag();
        lanCables.forEach((k, v) -> lan.putInt(Long.toString(k), v));
        tag.put("lan", lan);
        ListTag panelList = new ListTag();
        for (Panel p : panels) {
            CompoundTag c = new CompoundTag();
            c.putLong("pos", p.pos.asLong());
            c.putInt("facing", p.facing.get3DDataValue());
            panelList.add(c);
        }
        tag.put("panels", panelList);
        tag.putLongArray("fiberEnds", fiberEnds);
    }

    public List<Panel> panels() {
        return panels;
    }

    /** Cable blocks this piece places besides the buried village cable (packed pos -> arms). */
    public Map<Long, Integer> fixedCables() {
        return lanCables;
    }

    public int village() {
        return village;
    }

    /** The ISP router's fiber port exit toward the previous ({@code false}) or next village, or null. */
    public BlockPos fiberExit(boolean next) {
        return fiberEnds.length == 4 ? BlockPos.of(fiberEnds[next ? 2 : 0]) : null;
    }

    public List<Terminal> terminals() {
        return terminals;
    }

    public List<Node> nodes() {
        return nodes;
    }

    private int y(WorldGenLevel level, Node n, Map<Integer, Integer> cache, int index) {
        if (n.fixedY != SURFACE) return n.fixedY;
        // Same heightmap as the terrain-matching streets (GravityProcessor maps WG to the
        // live heightmap when placing straight into a ServerLevel, e.g. /place).
        Heightmap.Types type = level instanceof net.minecraft.server.level.ServerLevel
                ? Heightmap.Types.WORLD_SURFACE : Heightmap.Types.WORLD_SURFACE_WG;
        return cache.computeIfAbsent(index, i -> level.getHeight(type, n.x, n.z) - 2);
    }

    @Override
    public void postProcess(WorldGenLevel level, StructureManager structures, ChunkGenerator generator,
                            RandomSource random, BoundingBox box, ChunkPos chunk, BlockPos pivot) {
        Map<Integer, Integer> heights = new HashMap<>();
        BlockState cable = ModBlocks.NETWORK_CABLE.get().defaultBlockState();
        for (int i = 0; i < nodes.size(); i++) {
            Node n = nodes.get(i);
            if (n.x < box.minX() || n.x > box.maxX() || n.z < box.minZ() || n.z > box.maxZ()) continue;
            int y = y(level, n, heights, i);
            int lo = y;
            Map<Direction, Integer> links = new EnumMap<>(Direction.class);
            for (int j : adjacency[i]) {
                Node m = nodes.get(j);
                int yj = y(level, m, heights, j);
                lo = Math.min(lo, yj);
                Direction d = Direction.fromDelta(m.x - n.x, 0, m.z - n.z);
                if (d != null) links.put(d, Math.min(y, yj));
            }
            int hi = n.dropTop != SURFACE ? Math.max(y, n.dropTop) : y;
            for (int yy = lo; yy <= hi; yy++) {
                BlockPos p = new BlockPos(n.x, yy, n.z);
                if (!replaceable(level.getBlockState(p))) continue;
                BlockState s = cable.setValue(NetworkCableBlock.UP, yy < hi).setValue(NetworkCableBlock.DOWN, yy > lo);
                if (n.dropTop != SURFACE && yy == hi) s = s.setValue(NetworkCableBlock.UP, true);
                for (var e : links.entrySet())
                    if (e.getValue() == yy) s = s.setValue(NetworkCableBlock.getPropertyForDirection(e.getKey()), true);
                level.setBlock(p, s, 2);
            }
        }
        for (var e : lanCables.entrySet()) {
            BlockPos p = BlockPos.of(e.getKey());
            if (!box.isInside(p)) continue;
            BlockState s = cable;
            for (Direction d : Direction.values())
                if ((e.getValue() & (1 << d.ordinal())) != 0) s = s.setValue(NetworkCableBlock.getPropertyForDirection(d), true);
            level.setBlock(p, s, 2);
        }
        for (Panel panel : panels) {
            if (!box.isInside(panel.pos)) continue;
            level.setBlock(panel.pos, ModBlocks.FIBER_PATCH_PANEL.get().defaultBlockState()
                    .setValue(FiberPatchPanelBlock.FACING, panel.facing).setValue(NetworkCableBlock.DOWN, true), 2);
        }
        if (fiberEnds.length == 4 && (box.isInside(BlockPos.of(fiberEnds[0])) || box.isInside(BlockPos.of(fiberEnds[2])))) {
            var data = WorldNetwork.get(level.getLevel());
            data.recordEnd(village, false, BlockPos.of(fiberEnds[0]), BlockPos.of(fiberEnds[1]));
            data.recordEnd(village, true, BlockPos.of(fiberEnds[2]), BlockPos.of(fiberEnds[3]));
        }
        for (Terminal t : terminals) {
            if (!box.isInside(t.pos)) continue;
            level.setBlock(t.pos, ModBlocks.TERMINAL_BLOCK.get().defaultBlockState().setValue(TerminalBlock.FACING, t.facing), 2);
            var server = level.getLevel();
            var data = WorldNetwork.get(server);
            data.plan(server);
            data.provision(server, village, t.role);
            if (level.getBlockEntity(t.pos) instanceof TerminalBlockEntity be) {
                CompoundTag tag = new CompoundTag();
                tag.putUUID("computerId", data.identity(server, village, t.role));
                tag.putBoolean("wasRunning", true);
                tag.putString("wasmModule", "terminal_os.wasm");
                be.loadWithComponents(tag, level.registryAccess());
            }
        }
    }

    private static boolean replaceable(BlockState s) {
        return !s.is(Blocks.BEDROCK) && !(s.getBlock() instanceof TerminalBlock) && !(s.getBlock() instanceof InterfaceBlock);
    }

    @Override
    public BoundingBox getBeardifierBox() {
        return getBoundingBox();
    }

    @Override
    public TerrainAdjustment getTerrainAdjustment() {
        return TerrainAdjustment.NONE;
    }

    @Override
    public int getGroundLevelDelta() {
        return 0;
    }
}
//?}
