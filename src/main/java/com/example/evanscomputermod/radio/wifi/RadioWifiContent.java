package com.example.evanscomputermod.radio.wifi;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.world.item.Item;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** The Wi-Fi module (wlan0 SoftMAC radio; also a Wireless Controller receiver in controller mode). */
public final class RadioWifiContent {
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<Item> WIFI_MODULE = ITEMS.registerItem("wifi_module",
            props -> new WifiModuleItem(props.stacksTo(1)));

    private RadioWifiContent() {}

    public static void register(IEventBus modBus) {
        ITEMS.register(modBus);
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(WIFI_MODULE.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(
                WifiModule.TYPE, "Wi-Fi module: 2.4/5 GHz radio (wlan0), or a Wireless Controller receiver",
                WifiModule.class)));
    }
}
//?}
