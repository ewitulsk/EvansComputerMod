package com.example.evanscomputermod.radio.api;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.Direction;
import net.neoforged.neoforge.capabilities.BlockCapability;
import org.jetbrains.annotations.Nullable;

/**
 * Capabilities other mods use to find (or provide) radio hardware. Our access
 * points, SDRs, receivers and dishes expose {@link #ENDPOINT}; a mod adding its
 * own radio block can expose it too and register the endpoint with
 * {@code RadioMediumHooks.medium()}.
 */
public final class RadioCapabilities {

    /** The block's radio endpoint (its antenna on the shared medium). */
    public static final BlockCapability<RadioEndpoint, @Nullable Direction> ENDPOINT =
            BlockCapability.createSided(EvansComputerMod.id("radio_endpoint"), RadioEndpoint.class);

    private RadioCapabilities() {}
}
//?}
