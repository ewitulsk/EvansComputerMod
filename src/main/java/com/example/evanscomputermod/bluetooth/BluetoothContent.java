package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.Rarity;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** Registration of the AirPods, their case, the Bluetooth packets and the headsets' server tick (docs/BLUETOOTH.md). */
public final class BluetoothContent {

    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<AirPodsItem> AIRPODS =
            ITEMS.register("airpods", () -> new AirPodsItem(new Item.Properties().stacksTo(1).rarity(Rarity.UNCOMMON)));
    public static final DeferredItem<AirPodsCaseItem> AIRPODS_CASE =
            ITEMS.register("airpods_case", () -> new AirPodsCaseItem(new Item.Properties().stacksTo(1)));

    private BluetoothContent() {}

    public static void register(IEventBus modBus) {
        ITEMS.register(modBus);
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) {
                e.accept(AIRPODS.get());
                e.accept(AIRPODS_CASE.get());
            }
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() ->
                PeripheralTypes.register(BluetoothPeripheral.TYPE, "Bluetooth: scan, pair and connect headsets (AirPods)",
                        BluetoothPeripheral.class)));
        modBus.addListener(BluetoothContent::onPayloads);
        NeoForge.EVENT_BUS.addListener(Headsets::onServerTick);
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.bluetooth.client.BluetoothClient.register();
    }

    private static void onPayloads(RegisterPayloadHandlersEvent e) {
        var r = e.registrar("1");
        r.playToClient(BluetoothPackets.EarAudio.TYPE, BluetoothPackets.EarAudio.STREAM_CODEC,
                (p, ctx) -> ctx.enqueueWork(() -> com.example.evanscomputermod.speaker.client.EarbudsAudioClient.onAudio(p)));
    }
}
//?}
