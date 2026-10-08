package com.example.evanscomputermod.radio.antenna;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModCreativeTabs;
import com.example.evanscomputermod.radio.conductor.CoaxBlock;
import com.example.evanscomputermod.radio.conductor.ConductorBlock;
import com.example.evanscomputermod.radio.conductor.ConductorBlockEntity;
import com.example.evanscomputermod.radio.conductor.CopperConductorBlock;
import com.example.evanscomputermod.radio.conductor.FeedPointBlock;
import com.example.evanscomputermod.radio.conductor.InsulatorBlock;
import com.example.evanscomputermod.radio.conductor.LatticeMastBlock;
import com.example.evanscomputermod.radio.conductor.RfConductorData;
import com.example.evanscomputermod.radio.conductor.RfWrenchItem;
import net.minecraft.core.registries.Registries;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.Item;
import net.minecraft.world.level.block.Block;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.minecraft.world.level.material.MapColor;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;
import net.neoforged.neoforge.registries.datamaps.RegisterDataMapTypesEvent;

import java.util.List;
import java.util.function.Function;

/**
 * Phase 4 antennas (lanes 4A/4B/4C/4E/5A/4F): conductor tiers, insulator,
 * feed point, feedline blocks, the RF wrench and the antenna analyzer, the
 * {@code rf_conductor} data map and the antenna cache.
 */
public final class RadioAntennaContent {
    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    private static BlockBehaviour.Properties wire(SoundType sound, float strength) {
        return BlockBehaviour.Properties.of().mapColor(MapColor.METAL).strength(strength).sound(sound).noOcclusion()
                .forceSolidOff();
    }

    // ---- 4A conductor tiers (copper weathers; rod is aluminium, mast galvanized steel)
    public static final DeferredBlock<CopperConductorBlock> COPPER_WIRE = BLOCKS.registerBlock("copper_wire",
            p -> new CopperConductorBlock(p, 2, false), wire(SoundType.COPPER, 0.3f).noCollission().randomTicks());
    public static final DeferredBlock<CopperConductorBlock> ANTENNA_WIRE = BLOCKS.registerBlock("antenna_wire",
            p -> new CopperConductorBlock(p, 3, false), wire(SoundType.COPPER, 0.4f).noCollission().randomTicks());
    public static final DeferredBlock<CopperConductorBlock> HEAVY_CABLE = BLOCKS.registerBlock("heavy_cable",
            p -> new CopperConductorBlock(p, 5, true), wire(SoundType.COPPER, 1.0f).randomTicks());
    public static final DeferredBlock<ConductorBlock> ANTENNA_ROD = BLOCKS.registerBlock("antenna_rod",
            p -> new ConductorBlock(p, ConductorBlock.Role.CONDUCTOR, 7, true), wire(SoundType.METAL, 1.5f));
    public static final DeferredBlock<LatticeMastBlock> LATTICE_MAST = BLOCKS.registerBlock("lattice_mast",
            LatticeMastBlock::new, wire(SoundType.METAL, 3.0f).isViewBlocking((s, l, p) -> false));

    // ---- 4B support blocks
    public static final DeferredBlock<InsulatorBlock> INSULATOR = BLOCKS.registerBlock("insulator",
            InsulatorBlock::new, wire(SoundType.STONE, 1.0f).mapColor(MapColor.TERRACOTTA_WHITE));
    public static final DeferredBlock<FeedPointBlock> FEED_POINT = BLOCKS.registerBlock("feed_point",
            FeedPointBlock::new, wire(SoundType.METAL, 1.0f));
    public static final DeferredBlock<CoaxBlock> COAX_CABLE = BLOCKS.registerBlock("coax_cable",
            p -> new CoaxBlock(p, 4, false), wire(SoundType.WOOL, 0.3f).mapColor(MapColor.COLOR_BLACK));
    public static final DeferredBlock<CoaxBlock> HARDLINE = BLOCKS.registerBlock("hardline",
            p -> new CoaxBlock(p, 6, false), wire(SoundType.COPPER, 0.8f));
    public static final DeferredBlock<CoaxBlock> LIGHTNING_ARRESTOR = BLOCKS.registerBlock("lightning_arrestor",
            p -> new CoaxBlock(p, 7, true), wire(SoundType.METAL, 1.5f));

    public static final List<DeferredBlock<? extends ConductorBlock>> ALL_BLOCKS = List.of(COPPER_WIRE, ANTENNA_WIRE,
            HEAVY_CABLE, ANTENNA_ROD, LATTICE_MAST, INSULATOR, FEED_POINT, COAX_CABLE, HARDLINE, LIGHTNING_ARRESTOR);

    static {
        for (DeferredBlock<? extends ConductorBlock> b : ALL_BLOCKS) ITEMS.registerSimpleBlockItem(b);
    }

    public static final DeferredItem<RfWrenchItem> RF_WRENCH = ITEMS.registerItem("rf_wrench",
            (Function<Item.Properties, RfWrenchItem>) p -> new RfWrenchItem(p.stacksTo(1)));
    public static final DeferredItem<AntennaAnalyzerItem> ANTENNA_ANALYZER = ITEMS.registerItem("antenna_analyzer",
            (Function<Item.Properties, AntennaAnalyzerItem>) p -> new AntennaAnalyzerItem(p.stacksTo(1)));

    @SuppressWarnings("DataFlowIssue")
    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<ConductorBlockEntity>> CONDUCTOR_BE =
            BLOCK_ENTITIES.register("rf_conductor", () -> BlockEntityType.Builder.of(ConductorBlockEntity::new,
                    ALL_BLOCKS.stream().map(DeferredBlock::get).toArray(Block[]::new)).build(null));

    private RadioAntennaContent() {}

    public static void register(IEventBus modBus) {
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterDataMapTypesEvent e) -> e.register(RfConductorData.TYPE));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() != ModCreativeTabs.TAB.getKey()) return;
            for (DeferredBlock<? extends ConductorBlock> b : ALL_BLOCKS) e.accept(b.get());
            e.accept(RF_WRENCH.get());
            e.accept(ANTENNA_ANALYZER.get());
        });
        AntennaManager.register();
    }
}
//?}
