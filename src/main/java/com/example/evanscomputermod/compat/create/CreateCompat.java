package com.example.evanscomputermod.compat.create;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Create integration: the Redstone Link module and the Redstone Link
 * Interface block. Only classloaded when Create is installed
 * ({@link #register} is called behind a {@code ModList} check), so nothing
 * here is registered without it; the matching recipes carry a
 * {@code neoforge:mod_loaded} condition.
 */
public final class CreateCompat {

    public static final String MOD_ID = "create";

    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    public static final DeferredItem<Item> REDSTONE_LINK_MODULE = ITEMS.registerItem("redstone_link_module",
            props -> new RedstoneLinkModuleItem(props.stacksTo(1)));

    public static final DeferredBlock<RedstoneLinkInterfaceBlock> REDSTONE_LINK_INTERFACE =
            BLOCKS.registerBlock("redstone_link_interface", RedstoneLinkInterfaceBlock::new,
                    BlockBehaviour.Properties.of()
                            .strength(1.5f, 6.0f)
                            .sound(SoundType.METAL)
                            .noOcclusion());

    public static final DeferredItem<BlockItem> REDSTONE_LINK_INTERFACE_ITEM =
            ITEMS.registerSimpleBlockItem("redstone_link_interface", REDSTONE_LINK_INTERFACE);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<RedstoneLinkInterfaceBlockEntity>> REDSTONE_LINK_INTERFACE_BE =
            BLOCK_ENTITIES.register("redstone_link_interface", () -> BlockEntityType.Builder
                    .<RedstoneLinkInterfaceBlockEntity>of(RedstoneLinkInterfaceBlockEntity::new, REDSTONE_LINK_INTERFACE.get())
                    .build(null));

    private CreateCompat() {
    }

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener(CreateCompat::onRegisterCapabilities);
        modBus.addListener(CreateCompat::onCreativeTab);
        modBus.addListener(CreateCompat::onCommonSetup);
        EvansComputerMod.LOGGER.info("Create found: Redstone Link module and interface enabled");
    }

    private static void onRegisterCapabilities(RegisterCapabilitiesEvent event) {
        event.registerBlockEntity(PeripheralCapability.PERIPHERAL, REDSTONE_LINK_INTERFACE_BE.get(),
                (be, side) -> be.getPeripheral());
    }

    private static void onCreativeTab(BuildCreativeModeTabContentsEvent event) {
        if (event.getTabKey() == ModCreativeTabs.TAB.getKey()) {
            event.accept(REDSTONE_LINK_MODULE.get());
            event.accept(REDSTONE_LINK_INTERFACE_ITEM.get());
        }
    }

    private static void onCommonSetup(FMLCommonSetupEvent event) {
        PeripheralTypes.register(RedstoneLinkPeripheral.TYPE, "Create Redstone Link (64 channels)", RedstoneLinkPeripheral.class);
    }
}
//?}
