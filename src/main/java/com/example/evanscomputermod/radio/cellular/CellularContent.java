package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;


/**
 * Cellular hardware (Cellular v2, docs/gui/CELLULAR_V2.md): SIM cards, the LTE and
 * mmWave Modem modules (framed radios that {@code celld} drives) and the LTE Sector /
 * mmWave Array antennas a cell site's Advanced SDR feeds. Carriers, cell sites and
 * subscribers are programs on computers ({@code cored}, {@code celld-site}).
 */
public final class CellularContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /**
     * A written SIM's public identity (carrier, IMSI): saved and synced, shown in tooltips.
     * Blank cards have none.
     */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<com.example.evanscomputermod.card.SimIdentity>> SIM_CARD_DATA =
            COMPONENTS.register("sim_card", com.example.evanscomputermod.card.SimIdentity::componentType);

    /**
     * A written SIM's secret key: saved with the item, never sent to a client (the network
     * codec writes nothing and clients decode SimSecret.HIDDEN; see SimSecret).
     */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<com.example.evanscomputermod.card.SimSecret>> SIM_SECRET =
            COMPONENTS.register("sim_secret", com.example.evanscomputermod.card.SimSecret::componentType);

    private static BlockBehaviour.Properties metal() {
        return BlockBehaviour.Properties.of().strength(2f, 6f).sound(SoundType.METAL).noOcclusion().requiresCorrectToolForDrops();
    }

    public static final DeferredBlock<CellAntennaBlock> LTE_ANTENNA = BLOCKS.registerBlock("lte_antenna",
            p -> new CellAntennaBlock(PhyProfile.LTE, p), metal());
    public static final DeferredBlock<CellAntennaBlock> MMWAVE_ANTENNA = BLOCKS.registerBlock("mmwave_antenna",
            p -> new CellAntennaBlock(PhyProfile.MMWAVE, p), metal());

    public static final DeferredItem<BlockItem> LTE_ANTENNA_ITEM = ITEMS.registerSimpleBlockItem("lte_antenna", LTE_ANTENNA);
    public static final DeferredItem<BlockItem> MMWAVE_ANTENNA_ITEM = ITEMS.registerSimpleBlockItem("mmwave_antenna", MMWAVE_ANTENNA);
    public static final DeferredItem<Item> SIM_CARD = ITEMS.registerItem("sim_card", p -> new SimCardItem(p.stacksTo(16)));
    public static final DeferredItem<Item> LTE_MODEM = ITEMS.registerItem("lte_modem", p -> new ModemModuleItem(PhyProfile.LTE, p.stacksTo(1)));
    public static final DeferredItem<Item> MMWAVE_MODEM = ITEMS.registerItem("mmwave_modem", p -> new ModemModuleItem(PhyProfile.MMWAVE, p.stacksTo(1)));

    private CellularContent() {}

    public static Item modem(PhyProfile t) {
        return t == PhyProfile.LTE ? LTE_MODEM.get() : MMWAVE_MODEM.get();
    }

    public static Block antenna(PhyProfile t) {
        return t == PhyProfile.LTE ? LTE_ANTENNA.get() : MMWAVE_ANTENNA.get();
    }

    public static void register(IEventBus modBus) {
        COMPONENTS.register(modBus);
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            e.accept(LTE_ANTENNA_ITEM.get());
            e.accept(MMWAVE_ANTENNA_ITEM.get());
            e.accept(LTE_MODEM.get());
            e.accept(MMWAVE_MODEM.get());
            e.accept(SIM_CARD.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> {
            PeripheralTypes.register(FramedModemModule.TYPE, "Framed PHY radio (an LTE / mmWave Modem module, driven by celld): frames"
                    + " with measurements through /dev/frames.<bay>; set_role, set_channel, set_power, framed_status, peers", FramedModemModule.class);
        }));
    }
}
//?}
