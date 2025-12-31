package com.example.customworld.world;

import com.example.customworld.CustomWorldMod;
import com.mojang.serialization.MapCodec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Holder;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.level.WorldGenRegion;
import net.minecraft.world.level.LevelHeightAccessor;
import net.minecraft.world.level.NoiseColumn;
import net.minecraft.world.level.StructureManager;
import net.minecraft.world.level.biome.Biome;
import net.minecraft.world.level.biome.BiomeManager;
import net.minecraft.world.level.biome.Biomes;
import net.minecraft.world.level.biome.FixedBiomeSource;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.chunk.ChunkAccess;
import net.minecraft.world.level.chunk.ChunkGenerator;
import net.minecraft.world.level.levelgen.GenerationStep;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.RandomState;
import net.minecraft.world.level.levelgen.blending.Blender;

import java.util.List;
import java.util.concurrent.CompletableFuture;

/**
 * A custom chunk generator that creates a very distinctive terrain pattern:
 * - Checkerboard pattern of different colored blocks at the base
 * - Pillars of glass and glowstone rising at regular intervals
 * - Sea of lava in the lowlands
 * 
 * This is obviously not normal terrain generation!
 */
public class CustomChunkGenerator extends ChunkGenerator {

    public static final MapCodec<CustomChunkGenerator> CODEC = RecordCodecBuilder.mapCodec(instance ->
            instance.group(
                    Biome.CODEC.fieldOf("biome").forGetter(gen -> gen.biome)
            ).apply(instance, CustomChunkGenerator::new)
    );

    private final Holder<Biome> biome;

    public CustomChunkGenerator(Holder<Biome> biome) {
        super(new FixedBiomeSource(biome));
        this.biome = biome;
    }

    @Override
    protected MapCodec<? extends ChunkGenerator> codec() {
        return CODEC;
    }

    @Override
    public void applyCarvers(WorldGenRegion level, long seed, RandomState random, BiomeManager biomeManager, 
                             StructureManager structureManager, ChunkAccess chunk, GenerationStep.Carving step) {
        // No carvers needed for this simple generator
    }

    @Override
    public void buildSurface(WorldGenRegion level, StructureManager structureManager, RandomState random, ChunkAccess chunk) {
        // Surface building is done in fillFromNoise
    }

    @Override
    public void spawnOriginalMobs(WorldGenRegion level) {
        // No mob spawning
    }

    @Override
    public int getGenDepth() {
        return 384;
    }

    @Override
    public CompletableFuture<ChunkAccess> fillFromNoise(Blender blender, RandomState randomState, 
                                                         StructureManager structureManager, ChunkAccess chunk) {
        return CompletableFuture.supplyAsync(() -> {
            BlockPos.MutableBlockPos mutablePos = new BlockPos.MutableBlockPos();
            int minY = chunk.getMinBuildHeight();
            int startX = chunk.getPos().getMinBlockX();
            int startZ = chunk.getPos().getMinBlockZ();

            for (int x = 0; x < 16; x++) {
                for (int z = 0; z < 16; z++) {
                    int worldX = startX + x;
                    int worldZ = startZ + z;

                    // Create a base layer with bedrock
                    mutablePos.set(worldX, minY, worldZ);
                    chunk.setBlockState(mutablePos, Blocks.BEDROCK.defaultBlockState(), false);

                    // Create a checkerboard pattern base (y = minY+1 to minY+5)
                    boolean isBlack = ((worldX / 4) + (worldZ / 4)) % 2 == 0;
                    BlockState checkerBlock = isBlack ? 
                            Blocks.BLACK_CONCRETE.defaultBlockState() : 
                            Blocks.WHITE_CONCRETE.defaultBlockState();

                    for (int y = minY + 1; y <= minY + 5; y++) {
                        mutablePos.set(worldX, y, worldZ);
                        chunk.setBlockState(mutablePos, checkerBlock, false);
                    }

                    // Fill with lava from y=minY+6 to y=minY+20
                    for (int y = minY + 6; y <= minY + 20; y++) {
                        mutablePos.set(worldX, y, worldZ);
                        chunk.setBlockState(mutablePos, Blocks.LAVA.defaultBlockState(), false);
                    }

                    // Create tall pillars every 8 blocks in a grid pattern
                    if (worldX % 8 == 0 && worldZ % 8 == 0) {
                        // Determine pillar type based on position
                        boolean isGlowstone = ((worldX / 8) + (worldZ / 8)) % 2 == 0;
                        BlockState pillarBlock = isGlowstone ? 
                                Blocks.GLOWSTONE.defaultBlockState() : 
                                Blocks.GLASS.defaultBlockState();

                        // Build pillar from base to varying heights
                        int pillarHeight = 64 + ((worldX + worldZ) % 32);
                        for (int y = minY + 1; y <= minY + pillarHeight; y++) {
                            mutablePos.set(worldX, y, worldZ);
                            chunk.setBlockState(mutablePos, pillarBlock, false);
                        }

                        // Top the pillar with a beacon or sea lantern
                        mutablePos.set(worldX, minY + pillarHeight + 1, worldZ);
                        chunk.setBlockState(mutablePos, Blocks.SEA_LANTERN.defaultBlockState(), false);
                    }

                    // Create floating islands using sin/cos pattern
                    double islandNoise = Math.sin(worldX * 0.05) * Math.cos(worldZ * 0.05);
                    if (islandNoise > 0.7) {
                        int islandY = 100 + (int)(islandNoise * 20);
                        // Create a small floating island
                        for (int iy = -2; iy <= 0; iy++) {
                            int radius = 2 - Math.abs(iy);
                            for (int ix = -radius; ix <= radius; ix++) {
                                for (int iz = -radius; iz <= radius; iz++) {
                                    if (ix * ix + iz * iz <= radius * radius) {
                                        int bx = worldX + ix;
                                        int bz = worldZ + iz;
                                        // Only place if within chunk
                                        if (bx >= startX && bx < startX + 16 && 
                                            bz >= startZ && bz < startZ + 16) {
                                            mutablePos.set(bx, islandY + iy, bz);
                                            if (iy == 0) {
                                                chunk.setBlockState(mutablePos, Blocks.GRASS_BLOCK.defaultBlockState(), false);
                                            } else {
                                                chunk.setBlockState(mutablePos, Blocks.DIRT.defaultBlockState(), false);
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            }

            return chunk;
        });
    }

    @Override
    public int getSeaLevel() {
        return 63;
    }

    @Override
    public int getMinY() {
        return -64;
    }

    @Override
    public int getBaseHeight(int x, int z, Heightmap.Types type, LevelHeightAccessor level, RandomState random) {
        // Return base ground level
        return 64;
    }

    @Override
    public NoiseColumn getBaseColumn(int x, int z, LevelHeightAccessor level, RandomState random) {
        BlockState[] states = new BlockState[level.getHeight()];
        int minY = level.getMinBuildHeight();
        
        for (int i = 0; i < states.length; i++) {
            int y = minY + i;
            if (y == minY) {
                states[i] = Blocks.BEDROCK.defaultBlockState();
            } else if (y <= minY + 5) {
                states[i] = Blocks.BLACK_CONCRETE.defaultBlockState();
            } else if (y <= minY + 20) {
                states[i] = Blocks.LAVA.defaultBlockState();
            } else {
                states[i] = Blocks.AIR.defaultBlockState();
            }
        }
        
        return new NoiseColumn(minY, states);
    }

    @Override
    public void addDebugScreenInfo(List<String> info, RandomState random, BlockPos pos) {
        info.add("Custom World Generator - Checkerboard Pillars Dimension");
    }
}
