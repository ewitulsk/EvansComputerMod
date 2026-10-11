package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtUtils;
import net.minecraft.nbt.Tag;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.*;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.levelgen.structure.templatesystem.StructureTemplateManager;

import java.util.*;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Read-only view of a structure template's blocks in its own (unrotated) coordinates,
 * read through {@code StructureTemplate.save}. Cached per template location; used to
 * find the ISP's mast/panel/router and a free desk spot inside vanilla houses.
 */
public final class TemplateScan {
    private static final Map<ResourceLocation, Optional<TemplateScan>> CACHE = new ConcurrentHashMap<>();

    public final int sx, sy, sz;
    private final Map<BlockPos, BlockState> blocks = new HashMap<>();
    private final Map<BlockPos, CompoundTag> nbt = new HashMap<>();
    private volatile Optional<Desk> desk;

    /** A two-terminal desk: router at {@code router}, PC one block to the router's {@code side}. */
    public record Desk(BlockPos router, BlockPos pc, Direction facing) {}

    private TemplateScan(CompoundTag tag) {
        var size = tag.getList("size", Tag.TAG_INT);
        sx = size.getInt(0);
        sy = size.getInt(1);
        sz = size.getInt(2);
        var palette = tag.contains("palette") ? tag.getList("palette", Tag.TAG_COMPOUND)
                : tag.getList("palettes", Tag.TAG_LIST).getList(0);
        List<BlockState> states = new ArrayList<>();
        for (int i = 0; i < palette.size(); i++)
            states.add(NbtUtils.readBlockState(BuiltInRegistries.BLOCK.asLookup(), palette.getCompound(i)));
        var list = tag.getList("blocks", Tag.TAG_COMPOUND);
        for (int i = 0; i < list.size(); i++) {
            CompoundTag b = list.getCompound(i);
            var p = b.getList("pos", Tag.TAG_INT);
            BlockPos pos = new BlockPos(p.getInt(0), p.getInt(1), p.getInt(2));
            blocks.put(pos, states.get(b.getInt("state")));
            if (b.contains("nbt")) nbt.put(pos, b.getCompound("nbt"));
        }
    }

    public static Optional<TemplateScan> of(StructureTemplateManager manager, ResourceLocation location) {
        return CACHE.computeIfAbsent(location, l -> manager.get(l).map(t -> new TemplateScan(t.save(new CompoundTag()))));
    }

    public BlockState state(int x, int y, int z) {
        return blocks.getOrDefault(new BlockPos(x, y, z), Blocks.STRUCTURE_VOID.defaultBlockState());
    }

    public CompoundTag nbt(BlockPos pos) {
        return nbt.get(pos);
    }

    /** First block whose NBT carries {@code key = value} (e.g. ecmRole), else null. */
    public BlockPos findNbt(String key, String value) {
        return nbt.entrySet().stream().filter(e -> value.equals(e.getValue().getString(key)))
                .map(Map.Entry::getKey).sorted().findFirst().orElse(null);
    }

    /** Jigsaw blocks with the given name. */
    public List<BlockPos> jigsaws(String name) {
        List<BlockPos> out = new ArrayList<>();
        nbt.forEach((pos, tag) -> {
            if (blocks.get(pos).is(Blocks.JIGSAW) && name.equals(tag.getString("name"))) out.add(pos);
        });
        out.sort(Comparator.naturalOrder());
        return out;
    }

    /** Highest y of the lattice mast on the template's centre column, or -1. */
    public int mastTop() {
        int top = -1;
        for (var e : blocks.entrySet()) {
            BlockPos p = e.getKey();
            if (p.getX() == sx / 2 && p.getZ() == sz / 2
                    && BuiltInRegistries.BLOCK.getKey(e.getValue().getBlock()).getPath().equals("lattice_mast"))
                top = Math.max(top, p.getY());
        }
        return top;
    }

    /** All positions holding {@code block}, sorted. */
    public List<BlockPos> findAll(Block block) {
        return blocks.entrySet().stream().filter(e -> e.getValue().is(block)).map(Map.Entry::getKey).sorted().toList();
    }

    /** Is the template cell empty (air) and inside the template? */
    public boolean isAir(int x, int y, int z) {
        return x >= 0 && y >= 0 && z >= 0 && x < sx && y < sy && z < sz && state(x, y, z).isAir();
    }

    public BlockPos find(Block block) {
        return blocks.entrySet().stream().filter(e -> e.getValue().is(block)).map(Map.Entry::getKey)
                .sorted().findFirst().orElse(null);
    }

    /** Cells at the street surface layer (y = 0) carrying a path block. */
    public List<int[]> roadCells() {
        List<int[]> out = new ArrayList<>();
        blocks.forEach((p, s) -> {
            if (p.getY() == 0 && (s.is(Blocks.DIRT_PATH) || s.is(Blocks.SMOOTH_SANDSTONE) || s.is(Blocks.TERRACOTTA)))
                out.add(new int[] {p.getX(), p.getZ()});
        });
        return out;
    }

    private boolean air(int x, int y, int z) {
        if (x < 0 || z < 0 || x >= sx || z >= sz || y < 0 || y >= sy) return false;
        BlockState s = state(x, y, z);
        return s.isAir() || s.is(Blocks.STRUCTURE_VOID);
    }

    private boolean solid(int x, int y, int z) {
        BlockState s = state(x, y, z);
        if (s.is(Blocks.JIGSAW)) return true;
        if (s.isAir() || s.is(Blocks.STRUCTURE_VOID) || s.is(Blocks.GLASS) || s.getBlock() instanceof LeavesBlock)
            return false;
        return s.isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
    }

    private boolean roofed(int x, int y, int z) {
        for (int yy = y + 2; yy < sy; yy++) if (!air(x, yy, z) && !state(x, yy, z).is(Blocks.STRUCTURE_VOID)) return true;
        return false;
    }

    /** The cell's floor is at y - 1 with no hollow beneath it inside the template. */
    private boolean grounded(int x, int y, int z) {
        if (!solid(x, y - 1, z)) return false;
        for (int yy = 0; yy < y - 1; yy++) if (state(x, yy, z).isAir()) return false;
        return true;
    }

    /**
     * A free spot for a router and a PC side by side against a wall: both cells and the
     * cell above them (where the patch cable runs) empty and indoors, on a solid ground
     * floor, the cells in front walkable, not next to a door and not on a villager spawn.
     * Deterministic: the best-scoring spot, ties broken by position.
     */
    public Optional<Desk> desk() {
        if (desk != null) return desk;
        Set<Long> spawns = new HashSet<>();
        Set<Long> doors = new HashSet<>();
        blocks.forEach((p, s) -> {
            if (s.is(Blocks.JIGSAW) && nbt.containsKey(p) && "minecraft:bottom".equals(nbt.get(p).getString("name")))
                spawns.add(BlockPos.asLong(p.getX(), 0, p.getZ()));
            if (s.getBlock() instanceof DoorBlock) doors.add(BlockPos.asLong(p.getX(), 0, p.getZ()));
        });
        Desk best = null;
        int bestScore = Integer.MIN_VALUE;
        for (int y = 1; y < sy - 2; y++)
            for (int x = 0; x < sx; x++)
                for (int z = 0; z < sz; z++)
                    for (Direction f : Direction.Plane.HORIZONTAL) {
                        Direction side = f.getClockWise();
                        int[][] cells = {{x, z}, {x + side.getStepX(), z + side.getStepZ()}};
                        boolean ok = true;
                        int wall = 0;
                        for (int[] c : cells) {
                            int cx = c[0], cz = c[1], fx = cx + f.getStepX(), fz = cz + f.getStepZ();
                            if (!(air(cx, y, cz) && air(cx, y + 1, cz) && grounded(cx, y, cz) && roofed(cx, y, cz))
                                    || spawns.contains(BlockPos.asLong(cx, 0, cz))
                                    || !(air(fx, y, fz) && air(fx, y + 1, fz) && solid(fx, y - 1, fz))) {
                                ok = false;
                                break;
                            }
                            for (int dx = -1; dx <= 1 && ok; dx++)
                                for (int dz = -1; dz <= 1 && ok; dz++)
                                    if (Math.abs(dx) + Math.abs(dz) <= 1 && doors.contains(BlockPos.asLong(cx + dx, 0, cz + dz)))
                                        ok = false;
                            if (!ok) break;
                            if (solid(cx - f.getStepX(), y, cz - f.getStepZ())) wall++;
                        }
                        if (!ok) continue;
                        int score = wall * 1000 - y * 100;
                        if (score > bestScore) {
                            bestScore = score;
                            best = new Desk(new BlockPos(x, y, z), new BlockPos(cells[1][0], y, cells[1][1]), f);
                        }
                    }
        desk = Optional.ofNullable(best);
        return desk;
    }
}
//?}
