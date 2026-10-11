package com.example.evanscomputermod.radio.hazard;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.world.item.Item;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Radio hazards (lane 5C): gamerules, the RF meter, melted scrap, lightning
 * and RF exposure listeners, feed-point ownership. Registered from
 * {@code RadioAmpContent}.
 */
public final class RadioHazardContent {
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<RfMeterItem> RF_METER = ITEMS.register("rf_meter",
            () -> new RfMeterItem(new Item.Properties().stacksTo(1)));
    /** What a melted wire, coax or burnt-out amplifier leaves behind. */
    public static final DeferredItem<Item> MELTED_SCRAP = ITEMS.registerSimpleItem("melted_scrap");

    private RadioHazardContent() {}

    public static void register(IEventBus modBus) {
        ITEMS.register(modBus);
        RadioGameRules.register();
        RadioOwners.register();
        NeoForge.EVENT_BUS.addListener(LightningHazard::onJoin);
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) -> RfExposure.tick(e.getServer()));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(RF_METER.get());
        });
    }
}
//?}
