package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.LongArgumentType;
import com.mojang.brigadier.arguments.StringArgumentType;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.serialization.Codec;
import com.mojang.serialization.DataResult;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.coordinates.BlockPosArgument;
import net.minecraft.core.BlockPos;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.core.registries.Registries;
import net.minecraft.network.chat.Component;
import net.minecraft.network.codec.ByteBufCodecs;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.tags.TagKey;
import net.minecraft.world.inventory.MenuType;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.common.extensions.IMenuTypeExtension;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

import java.util.Locale;

/**
 * Cellular (GUI roadmap P7): SIM cards, the LTE and mmWave modem modules, the
 * Cell Tower base station and its two antennas, the carrier screen, and the
 * {@code /ecm cell} admin commands. docs/gui/CELLULAR.md describes the design.
 */
public final class CellularContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);
    private static final DeferredRegister<MenuType<?>> MENUS = DeferredRegister.create(Registries.MENU, EvansComputerMod.MODID);
    private static final DeferredRegister<DataComponentType<?>> COMPONENTS =
            DeferredRegister.create(Registries.DATA_COMPONENT_TYPE, EvansComputerMod.MODID);

    /** Mast blocks a Cell Tower stacks its antennas on. */
    public static final TagKey<Block> CELL_MASTS = TagKey.create(Registries.BLOCK, EvansComputerMod.id("cell_masts"));

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

    public static final DeferredBlock<CellTowerBlock> CELL_TOWER = BLOCKS.registerBlock("cell_tower", CellTowerBlock::new,
            metal().lightLevel(s -> s.getValue(CellTowerBlock.ACTIVE) ? 4 : 0));
    public static final DeferredBlock<CellAntennaBlock> LTE_ANTENNA = BLOCKS.registerBlock("lte_antenna",
            p -> new CellAntennaBlock(CellTech.LTE, p), metal());
    public static final DeferredBlock<CellAntennaBlock> MMWAVE_ANTENNA = BLOCKS.registerBlock("mmwave_antenna",
            p -> new CellAntennaBlock(CellTech.MMWAVE, p), metal());

    public static final DeferredItem<BlockItem> CELL_TOWER_ITEM = ITEMS.registerSimpleBlockItem("cell_tower", CELL_TOWER);
    public static final DeferredItem<BlockItem> LTE_ANTENNA_ITEM = ITEMS.registerSimpleBlockItem("lte_antenna", LTE_ANTENNA);
    public static final DeferredItem<BlockItem> MMWAVE_ANTENNA_ITEM = ITEMS.registerSimpleBlockItem("mmwave_antenna", MMWAVE_ANTENNA);
    public static final DeferredItem<Item> SIM_CARD = ITEMS.registerItem("sim_card", p -> new SimCardItem(p.stacksTo(16)));
    public static final DeferredItem<Item> LTE_MODEM = ITEMS.registerItem("lte_modem", p -> new ModemModuleItem(CellTech.LTE, p.stacksTo(1)));
    public static final DeferredItem<Item> MMWAVE_MODEM = ITEMS.registerItem("mmwave_modem", p -> new ModemModuleItem(CellTech.MMWAVE, p.stacksTo(1)));

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<CellTowerBlockEntity>> CELL_TOWER_BE =
            BLOCK_ENTITIES.register("cell_tower", () -> BlockEntityType.Builder.of(CellTowerBlockEntity::new, CELL_TOWER.get()).build(null));
    public static final DeferredHolder<MenuType<?>, MenuType<CellTowerMenu>> CELL_TOWER_MENU =
            MENUS.register("cell_tower", () -> IMenuTypeExtension.create(CellTowerMenu::new));

    private CellularContent() {}

    public static Item modem(CellTech t) {
        return t == CellTech.LTE ? LTE_MODEM.get() : MMWAVE_MODEM.get();
    }

    public static Block antenna(CellTech t) {
        return t == CellTech.LTE ? LTE_ANTENNA.get() : MMWAVE_ANTENNA.get();
    }

    public static void register(IEventBus modBus) {
        COMPONENTS.register(modBus);
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        MENUS.register(modBus);
        modBus.addListener((RegisterPayloadHandlersEvent e) -> CellPackets.register(e));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            e.accept(CELL_TOWER_ITEM.get());
            e.accept(LTE_ANTENNA_ITEM.get());
            e.accept(MMWAVE_ANTENNA_ITEM.get());
            e.accept(LTE_MODEM.get());
            e.accept(MMWAVE_MODEM.get());
            e.accept(SIM_CARD.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() -> {
            PeripheralTypes.register(ModemModule.TYPE_LTE, "LTE modem: cellular data (wwan0) on 700-900 MHz; status, cells, get_sim",
                    ModemModule.class);
            PeripheralTypes.register(ModemModule.TYPE_MMWAVE, "mmWave modem: 5G cellular data (wwan0) at 28 GHz, line of sight; status, cells, get_sim",
                    ModemModule.class);
        }));
        NeoForge.EVENT_BUS.addListener((RegisterCommandsEvent e) -> registerCommands(e.getDispatcher()));
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.radio.cellular.client.CellClient.register(modBus);
    }

    // ------------------------------------------------------------ /ecm cell

    private static void registerCommands(CommandDispatcher<CommandSourceStack> d) {
        d.register(Commands.literal("ecm").requires(s -> s.hasPermission(2))
                .then(Commands.literal("cell")
                        .then(Commands.literal("tower").then(Commands.argument("pos", BlockPosArgument.blockPos())
                                .executes(CellularContent::towerStatus)))
                        .then(Commands.literal("carriers").executes(CellularContent::listCarriers))
                        .then(Commands.literal("carrier").then(Commands.argument("id", StringArgumentType.word())
                                .executes(CellularContent::carrierStatus)
                                .then(Commands.literal("quota").then(Commands.argument("imsi", LongArgumentType.longArg(0))
                                        .then(Commands.argument("bytes", LongArgumentType.longArg(-1))
                                                .executes(CellularContent::setQuota))))))));
    }

    private static int towerStatus(CommandContext<CommandSourceStack> ctx) throws com.mojang.brigadier.exceptions.CommandSyntaxException {
        BlockPos pos = BlockPosArgument.getLoadedBlockPos(ctx, "pos");
        if (!(ctx.getSource().getLevel().getBlockEntity(pos) instanceof CellTowerBlockEntity be)) {
            ctx.getSource().sendFailure(Component.literal("No cell tower at " + pos.toShortString()));
            return 0;
        }
        for (String line : be.statusLines()) ctx.getSource().sendSuccess(() -> Component.literal(line), false);
        for (CellTech t : CellTech.values()) {
            TowerLink l = be.link(t);
            if (l == null) continue;
            for (TowerLink.DeviceStatus dev : l.devices())
                ctx.getSource().sendSuccess(() -> Component.literal(String.format(Locale.ROOT,
                        "    %s %s %s rnti %d slice %d: uplink %.1f dBm / %.1f dB, downlink CQI %d, %d B up, %d B down",
                        t.displayName, SimCard.formatImsi(dev.imsi()), dev.mac(), dev.rnti(), dev.slice(), dev.ulRssiDbm(), dev.ulSinrDb(),
                        dev.dlCqi(), dev.bytesUp(), dev.bytesDown())), false);
        }
        return 1;
    }

    private static int listCarriers(CommandContext<CommandSourceStack> ctx) {
        var reg = CarrierRegistry.get(ctx.getSource().getServer());
        if (reg.carriers().isEmpty()) ctx.getSource().sendSuccess(() -> Component.literal("No carriers yet"), false);
        for (CarrierRegistry.Carrier c : reg.carriers())
            ctx.getSource().sendSuccess(() -> Component.literal(c.id + " (#" + c.number + ", owner " + c.ownerName + "): "
                    + c.subscribers().size() + " SIMs"), false);
        return reg.carriers().size();
    }

    private static int carrierStatus(CommandContext<CommandSourceStack> ctx) {
        var reg = CarrierRegistry.get(ctx.getSource().getServer());
        CarrierRegistry.Carrier c = reg.carrier(StringArgumentType.getString(ctx, "id"));
        if (c == null) {
            ctx.getSource().sendFailure(Component.literal("No such carrier"));
            return 0;
        }
        ctx.getSource().sendSuccess(() -> Component.literal(c.id + " (#" + c.number + ", owner " + c.ownerName + ")"), false);
        for (CarrierRegistry.Subscriber s : c.subscribers()) {
            TowerLink t = reg.servingTower(s.imsi);
            ctx.getSource().sendSuccess(() -> Component.literal(String.format(Locale.ROOT, "  %s %s: %d B used of %s%s%s",
                    SimCard.formatImsi(s.imsi), s.label, s.usedBytes, s.quotaBytes < 0 ? "no limit" : s.quotaBytes + " B",
                    t == null ? "" : ", on " + t.tech().displayName, s.revoked ? ", revoked" : "")), false);
        }
        return 1;
    }

    private static int setQuota(CommandContext<CommandSourceStack> ctx) {
        var reg = CarrierRegistry.get(ctx.getSource().getServer());
        CarrierRegistry.Carrier c = reg.carrier(StringArgumentType.getString(ctx, "id"));
        long imsi = LongArgumentType.getLong(ctx, "imsi"), bytes = LongArgumentType.getLong(ctx, "bytes");
        if (c == null || !reg.setQuota(c, imsi, bytes)) {
            ctx.getSource().sendFailure(Component.literal("No such carrier / SIM"));
            return 0;
        }
        ctx.getSource().sendSuccess(() -> Component.literal("Data limit of " + SimCard.formatImsi(imsi) + ": "
                + (bytes < 0 ? "none" : bytes + " B")), true);
        return 1;
    }
}
//?}
