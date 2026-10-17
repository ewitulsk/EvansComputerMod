package com.example.evanscomputermod.voice;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.api.peripheral.PeripheralCapability;
import com.example.evanscomputermod.api.peripheral.PeripheralTypes;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.core.registries.Registries;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.level.block.SoundType;
import net.minecraft.world.level.block.entity.BlockEntityType;
import net.minecraft.world.level.block.state.BlockBehaviour;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.event.lifecycle.FMLCommonSetupEvent;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.capabilities.RegisterCapabilitiesEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredBlock;
import net.neoforged.neoforge.registries.DeferredHolder;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** Registration of the Microphone block, the voice packets, and the voice server tick (docs/VOICE.md). */
public final class VoiceContent {

    private static final DeferredRegister.Blocks BLOCKS = DeferredRegister.createBlocks(EvansComputerMod.MODID);
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);
    private static final DeferredRegister<BlockEntityType<?>> BLOCK_ENTITIES =
            DeferredRegister.create(Registries.BLOCK_ENTITY_TYPE, EvansComputerMod.MODID);

    public static final DeferredBlock<MicrophoneBlock> MICROPHONE =
            BLOCKS.registerBlock("microphone", MicrophoneBlock::new,
                    BlockBehaviour.Properties.of().strength(1.0f, 3.0f).sound(SoundType.METAL).noOcclusion());

    public static final DeferredItem<BlockItem> MICROPHONE_ITEM = ITEMS.registerSimpleBlockItem("microphone", MICROPHONE);

    public static final DeferredHolder<BlockEntityType<?>, BlockEntityType<MicrophoneBlockEntity>> MICROPHONE_BE =
            BLOCK_ENTITIES.register("microphone", () ->
                    BlockEntityType.Builder.<MicrophoneBlockEntity>of(MicrophoneBlockEntity::new, MICROPHONE.get()).build(null));

    private VoiceContent() {}

    public static void register(IEventBus modBus, ModContainer container) {
        container.registerConfig(net.neoforged.fml.config.ModConfig.Type.CLIENT, VoiceClientConfig.SPEC, "evanscomputermod-voice-client.toml");
        BLOCKS.register(modBus);
        ITEMS.register(modBus);
        BLOCK_ENTITIES.register(modBus);
        modBus.addListener((RegisterCapabilitiesEvent e) ->
                e.registerBlockEntity(PeripheralCapability.PERIPHERAL, MICROPHONE_BE.get(), (be, side) -> be.getPeripheral()));
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(MICROPHONE_ITEM.get());
        });
        modBus.addListener((FMLCommonSetupEvent e) -> e.enqueueWork(() ->
                PeripheralTypes.register(MicrophonePeripheral.TYPE, "Microphone: PCM capture through /dev/mic",
                        MicrophonePeripheral.class)));
        modBus.addListener(VoiceContent::onPayloads);
        NeoForge.EVENT_BUS.addListener(VoiceServer::onServerTick);
        NeoForge.EVENT_BUS.addListener(VoiceServer::onLogout);
        if (FMLEnvironment.dist.isClient()) com.example.evanscomputermod.voice.client.VoiceClient.register(modBus, container);
    }

    private static void onPayloads(RegisterPayloadHandlersEvent e) {
        var r = e.registrar("1");
        r.playToServer(VoicePackets.Voice.TYPE, VoicePackets.Voice.STREAM_CODEC, (p, ctx) -> ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer player) VoiceServer.onVoice(player, p);
        }));
        r.playToServer(VoicePackets.Talk.TYPE, VoicePackets.Talk.STREAM_CODEC, (p, ctx) -> ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer player) VoiceServer.onTalk(player, p.down());
        }));
        r.playToClient(VoicePackets.Demand.TYPE, VoicePackets.Demand.STREAM_CODEC,
                (p, ctx) -> ctx.enqueueWork(() -> com.example.evanscomputermod.voice.client.VoiceClient.onDemand(p.flags())));
    }
}
//?}
