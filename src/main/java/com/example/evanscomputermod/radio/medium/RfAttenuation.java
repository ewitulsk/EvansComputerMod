package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.phys.Ground;
import com.example.evanscomputermod.radio.phys.MaterialAttenuation;
import com.example.evanscomputermod.radio.phys.Materials;
import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.core.BlockPos;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.BlockTags;
import net.minecraft.tags.FluidTags;
import net.minecraft.world.level.EmptyBlockGetter;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.registries.datamaps.DataMapType;

import java.util.Map;
import java.util.Optional;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The {@code evanscomputermod:rf_attenuation} block data map and the resolution
 * of a {@link BlockState} to an {@link RfBlock}.
 *
 * <p>Entry format ({@code data/<ns>/data_maps/block/rf_attenuation.json}, keys may be
 * block ids or {@code #tags}): either a material name, e.g. {@code "stone"}, or an object
 * <pre>{"material": "stone", "db_per_block": 33.0, "ref_mhz": 1000, "exponent": 0.78,
 *  "ground": "dry_ground", "metal": false, "fraction": 1.0}</pre>
 * where every field is optional and overrides the named material's default
 * ({@link Materials}): α(f) = db_per_block · (f / ref)^exponent per full block,
 * {@code ground} is the low-frequency/reflection medium ({@code sea_water},
 * {@code fresh_water}, {@code wet_ground}, {@code average_ground}, {@code dry_ground},
 * {@code dry_sand}, {@code ice}, {@code metal}), {@code fraction} the share of the
 * block that is material (panes, slabs).
 *
 * <p>Blocks without an entry fall back by sound type: glass, wood, metal (an iron
 * room is a Faraday cage), leaves, wool, sand/gravel, dirt, snow; water (also
 * waterlogged) is water; other full blocks are stone-like; blocks with no
 * collision (flowers, torches, rails) are air; other partial blocks count half.
 */
public final class RfAttenuation {

    public record Entry(Optional<String> material, Optional<Double> dbPerBlock, Optional<Double> refMhz,
                        Optional<Double> exponent, Optional<String> ground, Optional<Boolean> metal,
                        Optional<Double> fraction) {}

    private static final Codec<Entry> OBJECT = RecordCodecBuilder.create(i -> i.group(
            Codec.STRING.optionalFieldOf("material").forGetter(Entry::material),
            Codec.DOUBLE.optionalFieldOf("db_per_block").forGetter(Entry::dbPerBlock),
            Codec.DOUBLE.optionalFieldOf("ref_mhz").forGetter(Entry::refMhz),
            Codec.DOUBLE.optionalFieldOf("exponent").forGetter(Entry::exponent),
            Codec.STRING.optionalFieldOf("ground").forGetter(Entry::ground),
            Codec.BOOL.optionalFieldOf("metal").forGetter(Entry::metal),
            Codec.DOUBLE.optionalFieldOf("fraction").forGetter(Entry::fraction)
    ).apply(i, Entry::new));

    public static final Codec<Entry> CODEC = Codec.withAlternative(OBJECT, Codec.STRING.xmap(
            s -> new Entry(Optional.of(s), Optional.empty(), Optional.empty(), Optional.empty(), Optional.empty(),
                    Optional.empty(), Optional.empty()),
            e -> e.material().orElse("stone")));

    public static final DataMapType<Block, Entry> TYPE = DataMapType
            .builder(EvansComputerMod.id("rf_attenuation"), Registries.BLOCK, CODEC)
            .synced(CODEC, false)
            .build();

    private static final Map<BlockState, RfBlock> CACHE = new ConcurrentHashMap<>();

    private static final Map<String, Ground> GROUNDS = Map.of(
            "sea_water", Ground.SEA_WATER, "fresh_water", Ground.FRESH_WATER, "wet_ground", Ground.WET_GROUND,
            "average_ground", Ground.AVERAGE_GROUND, "dry_ground", Ground.DRY_GROUND, "dry_sand", Ground.DRY_SAND,
            "ice", Ground.ICE, "metal", Ground.METAL);

    private RfAttenuation() {}

    /** Forget resolved states (data maps reloaded). */
    public static void clearCache() {
        CACHE.clear();
    }

    public static RfBlock of(BlockState state) {
        RfBlock b = CACHE.get(state);
        if (b == null) {
            b = resolve(state);
            CACHE.put(state, b);
        }
        return b;
    }

    /** An entry turned into an RfBlock. */
    public static RfBlock fromEntry(Entry e) {
        String name = e.material().orElse("stone");
        MaterialAttenuation known = Materials.get(name);
        if (known == null) EvansComputerMod.LOGGER.warn("rf_attenuation: unknown material '{}', using stone", name);
        final MaterialAttenuation base = known == null ? Materials.STONE : known;
        MaterialAttenuation m = base;
        if (e.dbPerBlock().isPresent() || e.refMhz().isPresent() || e.exponent().isPresent() || e.ground().isPresent()) {
            Ground lf = e.ground().map(g -> GROUNDS.getOrDefault(g, base.lowFrequencyMedium())).orElse(base.lowFrequencyMedium());
            if (lf != null && lf.isPerfectConductor()) lf = null;   // metal handled as a reflector, not skin depth
            m = new MaterialAttenuation(e.dbPerBlock().orElse(base.dbPerBlockAtRef()),
                    e.refMhz().map(v -> v * 1e6).orElse(base.refFreqHz()), e.exponent().orElse(base.exponentK()), lf);
        }
        boolean metal = e.metal().orElse(name.equals("iron") || name.equals("copper") || "metal".equals(e.ground().orElse("")));
        Ground g = metal ? Ground.METAL : e.ground().map(GROUNDS::get).orElse(m.lowFrequencyMedium());
        return new RfBlock(name, m, g, metal, Math.max(0, Math.min(1, e.fraction().orElse(1.0))));
    }

    private static RfBlock resolve(BlockState state) {
        Entry e = state.getBlockHolder().getData(TYPE);
        if (e != null) return fromEntry(e);
        boolean water = state.getFluidState().is(FluidTags.WATER);
        if (state.isAir()) return water ? RfBlock.WATER : RfBlock.AIR;
        if (state.getFluidState().is(FluidTags.LAVA)) return RfBlock.STONE;
        if (state.is(BlockTags.LEAVES)) return RfBlock.LEAVES;
        SoundType s = state.getSoundType();
        double fraction = fraction(state);
        RfBlock base;
        if (s == SoundType.GLASS) base = RfBlock.GLASS;
        else if (s == SoundType.METAL || s == SoundType.ANVIL || s == SoundType.CHAIN || s == SoundType.NETHERITE_BLOCK
                || s == SoundType.HEAVY_CORE || s == SoundType.LANTERN || s == SoundType.VAULT || s == SoundType.TRIAL_SPAWNER) base = RfBlock.IRON;
        else if (s == SoundType.COPPER || s == SoundType.COPPER_BULB || s == SoundType.COPPER_GRATE) base = RfBlock.of("copper");
        else if (s == SoundType.WOOD || s == SoundType.BAMBOO_WOOD || s == SoundType.CHERRY_WOOD || s == SoundType.NETHER_WOOD
                || s == SoundType.STEM || s == SoundType.BAMBOO || s == SoundType.LADDER || s == SoundType.SCAFFOLDING) base = RfBlock.WOOD;
        else if (s == SoundType.WOOL) base = RfBlock.of("wool");
        else if (s == SoundType.SAND) base = RfBlock.of("sand");
        else if (s == SoundType.GRAVEL || s == SoundType.ROOTED_DIRT || s == SoundType.MUD || s == SoundType.GRASS && fraction >= 1) base = RfBlock.DIRT;
        else if (s == SoundType.SNOW || s == SoundType.POWDER_SNOW) base = RfBlock.of("snow");
        else if (s == SoundType.AZALEA_LEAVES || s == SoundType.CHERRY_LEAVES || s == SoundType.GRASS || s == SoundType.CROP
                || s == SoundType.VINE || s == SoundType.WET_GRASS) base = fraction > 0 ? RfBlock.LEAVES : RfBlock.AIR;
        else base = RfBlock.STONE;
        if (water && fraction < 1) return RfBlock.WATER;   // waterlogged: the water dominates
        if (fraction <= 0) return water ? RfBlock.WATER : RfBlock.AIR;
        return fraction >= 1 ? base : base.scaled(fraction);
    }

    /** Share of the block that is solid: 1 for full cubes, 0 for no collision, ½ otherwise. */
    private static double fraction(BlockState state) {
        try {
            var shape = state.getCollisionShape(EmptyBlockGetter.INSTANCE, BlockPos.ZERO);
            if (shape.isEmpty()) return state.is(BlockTags.LEAVES) ? 1 : 0;
            if (state.isCollisionShapeFullBlock(EmptyBlockGetter.INSTANCE, BlockPos.ZERO)) return 1;
            var b = shape.bounds();
            return Math.max(0.1, Math.min(1, (b.maxX - b.minX) * (b.maxY - b.minY) * (b.maxZ - b.minZ)));
        } catch (RuntimeException ex) {
            return 1;
        }
    }
}
//?}
