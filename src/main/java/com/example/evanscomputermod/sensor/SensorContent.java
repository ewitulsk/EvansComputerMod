package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.sensor.wire.BlockWireEntity;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import com.example.evanscomputermod.sensor.wire.WireConnection;
import com.example.evanscomputermod.sensor.wire.WirePackets;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.tags.TagKey;
import net.minecraft.world.InteractionResult;
import net.minecraft.world.entity.EntityType;
import net.minecraft.world.entity.MobCategory;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.material.PushReaction;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.event.entity.player.PlayerInteractEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/**
 * Lidar Sensor, Sensor Wire and the Wired Sensor Module (1.21.1). The wires
 * are a port of PowerGrid's block wires (Apache-2.0, see
 * META-INF/licenses/PowerGrid-NOTICE.txt).
 */
public final class SensorContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<EntityType<?>> ENTITIES =
            DeferredRegister.create(Registries.ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** Items that cut sensor wires (shears by default). */
    public static final TagKey<Item> WIRE_CUTTERS = TagKey.create(Registries.ITEM, EvansComputerMod.id("wire_cutters"));

    public static final DeferredItem<Item> SENSOR_WIRE = ITEMS.registerItem("sensor_wire",
            props -> new SensorWireItem(props.stacksTo(64)));

    public static final DeferredItem<Item> WIRED_SENSOR_MODULE = ITEMS.registerItem("wired_sensor_module",
            props -> new WiredSensorModuleItem(props.stacksTo(1)));

    public static final DeferredBlock<LidarSensorBlock> LIDAR_SENSOR = BLOCKS.registerBlock("lidar_sensor",
            LidarSensorBlock::new,
            BlockBehaviour.Properties.of()
                    .strength(0.8f, 3.0f)
                    .sound(SoundType.METAL)
                    .noOcclusion()
                    .noCollission()
                    .pushReaction(PushReaction.DESTROY));

    public static final DeferredItem<BlockItem> LIDAR_SENSOR_ITEM = ITEMS.registerSimpleBlockItem("lidar_sensor", LIDAR_SENSOR);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<LidarSensorBlockEntity>> LIDAR_SENSOR_BE =
            BLOCK_ENTITIES.register("lidar_sensor", () -> BlockEntityType.Builder
                    .of(LidarSensorBlockEntity::new, LIDAR_SENSOR.get())
                    .build(null));

    public static final DeferredHolder<EntityType<?>, EntityType<BlockWireEntity>> BLOCK_WIRE =
            ENTITIES.register("sensor_wire", () -> EntityType.Builder
                    .<BlockWireEntity>of(BlockWireEntity::new, MobCategory.MISC)
                    .sized(0.125f, 0.125f)
                    .clientTrackingRange(10)
                    .updateInterval(20)
                    .noSummon()
                    .fireImmune()
                    .build("sensor_wire"));

    /** The first end of a wire being placed. */
    public static final DeferredHolder<DataComponentType<?>, DataComponentType<WireConnection>> CONNECTION_DATA =
            COMPONENTS.register("wire_connection", () -> DataComponentType.<WireConnection>builder()
                    .persistent(WireConnection.CODEC)
                    .networkSynchronized(WireConnection.STREAM_CODEC)
                    .build());

    private SensorContent() {
    }

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        ENTITIES.register(modBus);
        COMPONENTS.register(modBus);
        modBus.addListener(SensorContent::onCreativeTab);
        modBus.addListener(SensorContent::onCommonSetup);
        modBus.addListener((RegisterPayloadHandlersEvent e) -> WirePackets.register(e));
        NeoForge.EVENT_BUS.addListener(SensorContent::onRightClickBlock);
        if(FMLEnvironment.dist.isClient())
            com.example.evanscomputermod.sensor.client.SensorClient.register(modBus);
    }

    private static void onCreativeTab(BuildCreativeModeTabContentsEvent event) {
        if(event.getTabKey() == ModCreativeTabs.TAB.getKey()) {
            event.accept(WIRED_SENSOR_MODULE.get());
            event.accept(LIDAR_SENSOR_ITEM.get());
            event.accept(SENSOR_WIRE.get());
        }
    }

    private static void onCommonSetup(FMLCommonSetupEvent event) {
        PeripheralTypes.register(WiredSensorModule.TYPE, "Wired sensors (lidar) on a bay module", WiredSensorModule.class);
    }

    /** Sensor Wire clicks go to the wire before the block's own use (e.g. the computer opening its screen). */
    private static void onRightClickBlock(PlayerInteractEvent.RightClickBlock event) {
        if(!(event.getItemStack().getItem() instanceof SensorWireItem))
            return;
        InteractionResult result = SensorWireItem.useOnBlock(event.getEntity(), event.getHand(), event.getPos(), event.getFace());
        if(result != InteractionResult.PASS) {
            event.setCanceled(true);
            event.setCancellationResult(result == InteractionResult.FAIL ? InteractionResult.FAIL
                    : InteractionResult.sidedSuccess(event.getLevel().isClientSide()));
        }
    }
}
//?}
