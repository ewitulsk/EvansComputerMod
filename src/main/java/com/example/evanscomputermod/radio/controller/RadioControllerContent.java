package com.example.evanscomputermod.radio.controller;

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

/** The Controller Receiver module (Wireless Controllers on the 2.4 GHz medium). */
public final class RadioControllerContent {
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<Item> CONTROLLER_RECEIVER_MODULE = ITEMS.registerItem("controller_receiver_module",
            props -> new ControllerReceiverModuleItem(props.stacksTo(1)));

    private RadioControllerContent() {}

    public static void register(IEventBus modBus) {
        ITEMS.register(modBus);
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(CONTROLLER_RECEIVER_MODULE.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(
                ControllerReceiverModule.TYPE, "Controller Receiver: Wireless Controllers on 2.4 GHz",
                ControllerReceiverModule.class)));
    }
}
//?}
