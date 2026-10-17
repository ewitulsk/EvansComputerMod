package com.example.evanscomputermod.disk;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.mojang.serialization.Codec;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.neoforge.capabilities.Capabilities;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Floppy disks and the Disk Drive (docs/CC_COMPAT.md, phase 4): a floppy's
 * files, in a drive next to a computer, are the folder {@code disk/}.
 */
public final class DiskContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** A floppy's number: its folder, {@code computer-data/disks/<n>/}. */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<Integer>> DISK_ID =
            COMPONENTS.register("disk_id", () -> DataComponentType.<Integer>builder()
                    .persistent(Codec.INT).networkSynchronized(ByteBufCodecs.VAR_INT).build());

    public static final DeferredItem<Item> FLOPPY_DISK = ITEMS.registerItem("floppy_disk", p -> new FloppyDiskItem(p.stacksTo(1)));

    public static final DeferredBlock<DiskDriveBlock> DISK_DRIVE = BLOCKS.registerBlock("disk_drive", DiskDriveBlock::new,
            BlockBehaviour.Properties.of().strength(2f, 6f).sound(SoundType.STONE).requiresCorrectToolForDrops());
    public static final DeferredItem<BlockItem> DISK_DRIVE_ITEM = ITEMS.registerSimpleBlockItem("disk_drive", DISK_DRIVE);
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<DiskDriveBlockEntity>> DISK_DRIVE_BE =
            BLOCK_ENTITIES.register("disk_drive", () -> BlockEntityType.Builder.of(DiskDriveBlockEntity::new, DISK_DRIVE.get()).build(null));

    private DiskContent() {}

    public static void register(IEventBus modBus) {
        COMPONENTS.register(modBus);
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) -> {
            e.registerBlockEntity(PeripheralCapability.PERIPHERAL, DISK_DRIVE_BE.get(), (be, side) -> be.peripheral());
            e.registerBlockEntity(Capabilities.ItemHandler.BLOCK, DISK_DRIVE_BE.get(), (be, side) -> be.automation());
        });
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            e.accept(DISK_DRIVE_ITEM.get());
            e.accept(FLOPPY_DISK.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> PeripheralTypes.register(DiskDrivePeripheral.TYPE,
                "Disk Drive: the floppy in it is the folder disk/ (is_disk_present, get_mount_path, get_disk_label,"
                        + " set_disk_label, get_disk_id, has_data, eject_disk)", DiskDrivePeripheral.class)));
        if (net.neoforged.fml.loading.FMLEnvironment.dist.isClient()
                && com.example.evanscomputermod.testing.RadioVisualLayout.suite().equals("disks"))
            net.neoforged.neoforge.common.NeoForge.EVENT_BUS.addListener(com.example.evanscomputermod.testing.client.DiskClientChecks::tick);
    }
}
//?}
