package com.example.evanscomputermod.radio.conductor;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.RadioContent;
import com.example.evanscomputermod.radio.antenna.graph.CoaxSpec;
import com.example.evanscomputermod.radio.antenna.graph.ConductorSpec;
import com.example.evanscomputermod.radio.antenna.graph.RfDefaults;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.state.BlockState;
import org.jetbrains.annotations.Nullable;

/**
 * Electrical lookups for blocks: the {@code rf_conductor} data map laid over
 * {@link RfDefaults}, plus copper oxidation and waterlogging.
 */
public final class RadioConductors {
    /** Blocks a coax run connects to besides feedline blocks: amplifiers, tuners, radios (other lanes add theirs). */
    public static final TagKey<Block> RF_COAX_PORTS = TagKey.create(Registries.BLOCK, EvansComputerMod.id("rf_coax_ports"));
    /** Water around a bare conductor: resistivity multiplier (lossy dielectric contact). */
    public static final double WATERLOGGED_LOSS = 20;

    private RadioConductors() {}

    @Nullable
    static RfConductorData data(BlockState state) {
        try {
            return state.getBlock().builtInRegistryHolder().getData(RfConductorData.TYPE);
        } catch (RuntimeException e) {
            return null;
        }
    }

    public static String id(BlockState state) {
        return BuiltInRegistries.BLOCK.getKey(state.getBlock()).toString();
    }

    /**
     * True if the block conducts RF: a bare conductor block or anything tagged {@code rf_conductors}
     * (and not {@code rf_insulators}, which wins).
     */
    public static boolean conducts(BlockState state) {
        return state.getBlock() instanceof ConductorBlock c ? c.role() == ConductorBlock.Role.CONDUCTOR
                : state.is(RadioContent.RF_CONDUCTORS) && !state.is(RadioContent.RF_INSULATORS);
    }

    /**
     * True if the block holds a wire mechanically but not electrically: anything tagged
     * {@code #evanscomputermod:rf_insulators} (the Insulator and feed points; a datapack can add
     * more, with a voltage rating from the {@code rf_conductor} data map).
     */
    public static boolean insulates(BlockState state) {
        return state.is(RadioContent.RF_INSULATORS);
    }

    /** The conductor spec of a block (data map over defaults), with oxidation and water applied. */
    public static ConductorSpec spec(BlockState state) {
        ConductorSpec base = RfDefaults.CONDUCTORS.getOrDefault(id(state), RfDefaults.METAL_BLOCK);
        RfConductorData d = data(state);
        ConductorSpec s = d == null ? base : d.over(base);
        if (state.hasProperty(CopperConductorBlock.OXIDATION)) s = s.oxidized(state.getValue(CopperConductorBlock.OXIDATION));
        if (state.hasProperty(ConductorBlock.WATERLOGGED) && state.getValue(ConductorBlock.WATERLOGGED))
            s = new ConductorSpec(s.name() + " (in water)", s.radius(), s.resistivity() * WATERLOGGED_LOSS, s.currentRatingA(),
                    s.coronaVoltage(), s.oxidizes());
        return s;
    }

    /** Peak voltage rating of an insulator or feed point. */
    public static double voltageRating(BlockState state) {
        double base = RfDefaults.INSULATORS.getOrDefault(id(state), RfDefaults.INSULATOR_VOLTS);
        RfConductorData d = data(state);
        return d == null ? base : d.voltageOver(base);
    }

    /** Feedline spec of a coax-family block. */
    public static CoaxSpec coax(BlockState state) {
        CoaxSpec base = RfDefaults.COAX.getOrDefault(id(state), RfDefaults.COAX_CABLE);
        RfConductorData d = data(state);
        return d == null ? base : d.over(base);
    }
}
//?}
