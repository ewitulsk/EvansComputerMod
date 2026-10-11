package com.example.evanscomputermod.worldgen;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.WorldNetwork;
import com.mojang.serialization.Codec;
import com.mojang.serialization.MapCodec;
import com.mojang.serialization.codecs.RecordCodecBuilder;

import net.minecraft.core.Holder;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.level.levelgen.Heightmap;
import net.minecraft.world.level.levelgen.VerticalAnchor;
import net.minecraft.world.level.levelgen.heightproviders.ConstantHeight;
import net.minecraft.world.level.levelgen.structure.*;
import net.minecraft.world.level.levelgen.structure.pieces.StructurePiecesBuilder;
import net.minecraft.world.level.levelgen.structure.pools.StructureTemplatePool;
import net.minecraft.world.level.levelgen.structure.structures.JigsawStructure;

import java.util.List;
import java.util.Optional;

/**
 * A vanilla jigsaw village grown from an ISP building, plus the village's computer
 * network. Generates only at its planned ring site and only for the style planned for
 * that site's biome, so exactly one of the five styled structures in the set wins.
 *
 * <p>The start piece is positioned by its {@code evanscomputermod:mast_anchor} jigsaw,
 * which lands on the site's (x, z); the mast is the template's centre column, so the
 * structure's random rotation never moves the fiber endpoint (see WorldNetwork.plan).
 */
public final class TechVillageStructure extends Structure {
    public static final MapCodec<TechVillageStructure> CODEC = RecordCodecBuilder.mapCodec(i -> i.group(
                    settingsCodec(i),
                    Codec.STRING.fieldOf("style").forGetter(s -> s.style),
                    StructureTemplatePool.CODEC.fieldOf("start_pool").forGetter(s -> s.startPool),
                    ResourceLocation.CODEC.fieldOf("start_jigsaw_name").forGetter(s -> s.anchor),
                    Codec.intRange(0, 20).fieldOf("size").forGetter(s -> s.size),
                    Codec.intRange(1, 116).fieldOf("max_distance_from_center").forGetter(s -> s.maxDistance))
            .apply(i, TechVillageStructure::new));

    private final String style;
    private final Holder<StructureTemplatePool> startPool;
    private final ResourceLocation anchor;
    private final int size, maxDistance;
    private final JigsawStructure jigsaw;

    public TechVillageStructure(StructureSettings settings, String style, Holder<StructureTemplatePool> startPool,
                                ResourceLocation anchor, int size, int maxDistance) {
        super(settings);
        this.style = style;
        this.startPool = startPool;
        this.anchor = anchor;
        this.size = size;
        this.maxDistance = maxDistance;
        // Same settings vanilla villages use (expansion hack, WORLD_SURFACE_WG start).
        this.jigsaw = new JigsawStructure(settings, startPool, Optional.of(anchor), size,
                ConstantHeight.of(VerticalAnchor.absolute(0)), true, Optional.of(Heightmap.Types.WORLD_SURFACE_WG),
                maxDistance, List.of(), JigsawStructure.DEFAULT_DIMENSION_PADDING, JigsawStructure.DEFAULT_LIQUID_SETTINGS);
    }

    public String style() {
        return style;
    }

    @Override
    public Optional<GenerationStub> findGenerationPoint(GenerationContext context) {
        WorldNetwork.Village site = WorldNetwork.siteAt(context.seed(), context.chunkPos().x, context.chunkPos().z);
        if (site == null || !style.equals(site.style())) return Optional.empty();
        return jigsaw.findGenerationPoint(context).map(stub -> new GenerationStub(stub.position(), builder -> {
            StructurePiecesBuilder inner = stub.getPiecesBuilder();
            List<StructurePiece> pieces = inner.build().pieces();
            pieces.forEach(builder::addPiece);
            VillageNetworkPlanner.plan(context.structureTemplateManager(), context.seed(), site, pieces)
                    .ifPresent(builder::addPiece);
        }));
    }

    @Override
    public StructureType<?> type() {
        return TechWorldgen.STRUCTURE.get();
    }
}
//?}
