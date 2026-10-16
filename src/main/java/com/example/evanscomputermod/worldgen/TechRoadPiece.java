package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import net.minecraft.core.BlockPos;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.NbtUtils;
import net.minecraft.tags.BlockTags;
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

/**
 * The roads {@link VillageRoadPlanner} planned from each building entrance to the streets.
 * Like the terrain-matching streets it follows the ground: in each cell it looks down from the
 * surface (the heightmap the streets use) past roof overhangs and leaves to the first natural
 * ground with room to walk above it, clears plants and snow off it and turns it into the
 * village's road block. Water and built floors are left alone. Declares no terrain adjustment.
 */
public final class TechRoadPiece extends StructurePiece
        implements net.neoforged.neoforge.common.world.PieceBeardifierModifier {
    /** How far below the surface (roofs, canopies) the ground is looked for. */
    private static final int DEPTH = 24;
    private final long[] cells;
    private final BlockState surface;

    public TechRoadPiece(long[] cells, BlockState surface, BoundingBox box) {
        super(TechWorldgen.ROAD_PIECE.get(), 0, box);
        this.cells = cells;
        this.surface = surface;
    }

    public TechRoadPiece(CompoundTag tag) {
        super(TechWorldgen.ROAD_PIECE.get(), tag);
        cells = tag.getLongArray("cells");
        BlockState s = NbtUtils.readBlockState(net.minecraft.core.registries.BuiltInRegistries.BLOCK.asLookup(),
                tag.getCompound("surface"));
        surface = s.isAir() ? Blocks.DIRT_PATH.defaultBlockState() : s;
    }

    @Override
    protected void addAdditionalSaveData(StructurePieceSerializationContext context, CompoundTag tag) {
        tag.putLongArray("cells", cells);
        tag.put("surface", NbtUtils.writeBlockState(surface));
    }

    /** Planned road cells, packed (x, 0, z). */
    public long[] cells() {
        return cells;
    }

    @Override
    public void postProcess(WorldGenLevel level, StructureManager structures, ChunkGenerator generator,
                            RandomSource random, BoundingBox box, ChunkPos chunk, BlockPos pivot) {
        // Same heightmap as the terrain-matching streets (see TechNetworkPiece).
        Heightmap.Types type = level instanceof net.minecraft.server.level.ServerLevel
                ? Heightmap.Types.WORLD_SURFACE : Heightmap.Types.WORLD_SURFACE_WG;
        for (long c : cells) {
            int x = BlockPos.getX(c), z = BlockPos.getZ(c);
            if (x < box.minX() || x > box.maxX() || z < box.minZ() || z > box.maxZ()) continue;
            // Down from the surface through roof overhangs, leaves and plants to the first
            // natural ground a player could stand on; built floors and water end the search.
            BlockPos p = new BlockPos(x, level.getHeight(type, x, z) - 1, z);
            for (int i = 0; i < DEPTH; i++, p = p.below()) {
                BlockState s = level.getBlockState(p);
                if (!s.getFluidState().isEmpty()) break;
                if (s.getCollisionShape(level, p).isEmpty()) continue;           // air, plants
                BlockPos up = p.above(), down = p.below();
                if (!level.getBlockState(up).getCollisionShape(level, up).isEmpty()) break;   // no room above: in the ground
                if (VillageRoadPlanner.isRoad(s)) break;
                if (!natural(s)) {
                    if (level.getBlockState(down).getCollisionShape(level, down).isEmpty()) continue;   // overhang, leaves
                    break;                                                       // a built floor
                }
                BlockState above = level.getBlockState(up);
                if (!above.isAir() && above.canBeReplaced() && above.getFluidState().isEmpty())
                    level.setBlock(up, Blocks.AIR.defaultBlockState(), 2);   // grass, flowers, snow layers
                level.setBlock(p, surface, 2);
                break;
            }
        }
    }

    private static boolean natural(BlockState s) {
        return s.is(BlockTags.DIRT) || s.is(BlockTags.SAND) || s.is(BlockTags.BASE_STONE_OVERWORLD) || s.is(BlockTags.TERRACOTTA)
                || s.is(Blocks.SNOW_BLOCK) || s.is(Blocks.GRAVEL) || s.is(Blocks.SANDSTONE) || s.is(Blocks.RED_SANDSTONE);
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
