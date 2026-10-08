package com.example.evanscomputermod.radio;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.neoforged.bus.api.IEventBus;

/**
 * Radio &amp; Wireless (1.21.1): the entry point every radio feature registers
 * through, so the mod constructor has one line. Each feature keeps its own
 * {@code DeferredRegister}s in its own {@code *Content} class (the
 * {@code SensorContent} pattern) and adds one call to {@link #register}.
 */
public final class RadioContent {

    /** Blocks that conduct RF and join antennas when touching a wire (iron bars, chains, lightning rods, metal blocks...). */
    public static final TagKey<Block> RF_CONDUCTORS = TagKey.create(Registries.BLOCK, EvansComputerMod.id("rf_conductors"));
    /** Blocks that connect mechanically but not electrically. */
    public static final TagKey<Block> RF_INSULATORS = TagKey.create(Registries.BLOCK, EvansComputerMod.id("rf_insulators"));
    /** Blocks that make a good RF ground under an antenna (water, metal roofs...). */
    public static final TagKey<Block> RF_GOOD_GROUND = TagKey.create(Registries.BLOCK, EvansComputerMod.id("rf_good_ground"));
    /** Items that cut/restore conductor connections (the wrench role). */
    public static final TagKey<Item> RF_WRENCHES = TagKey.create(Registries.ITEM, EvansComputerMod.id("rf_wrenches"));

    private RadioContent() {}

    public static void register(IEventBus modBus) {
        // One line per radio feature, in roadmap order.
        com.example.evanscomputermod.radio.medium.RadioMediumHooks.register(modBus);
        com.example.evanscomputermod.radio.power.RadioPowerContent.register(modBus);
        com.example.evanscomputermod.radio.controller.RadioControllerContent.register(modBus);
        com.example.evanscomputermod.radio.wifi.ap.AccessPointContent.register(modBus);
        com.example.evanscomputermod.radio.sdr.RadioSdrContent.register(modBus);
        com.example.evanscomputermod.radio.handheld.RadioHandheldContent.register(modBus);
        com.example.evanscomputermod.radio.antenna.RadioAntennaContent.register(modBus);
        com.example.evanscomputermod.radio.medium.WorldMediumContent.register(modBus);
        com.example.evanscomputermod.radio.microwave.MicrowaveContent.register(modBus);
        com.example.evanscomputermod.radio.wifi.RadioWifiContent.register(modBus);
    }
}
//?}
