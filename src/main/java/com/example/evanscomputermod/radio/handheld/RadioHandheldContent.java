package com.example.evanscomputermod.radio.handheld;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.ModCreativeTabs;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.Item;
import net.minecraft.world.item.ItemStack;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.loading.FMLEnvironment;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.BuildCreativeModeTabContentsEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.registries.DeferredItem;
import net.neoforged.neoforge.registries.DeferredRegister;

/** The handheld radio item, its packets and the server tick that runs it. */
public final class RadioHandheldContent {
    private static final DeferredRegister.Items ITEMS = DeferredRegister.createItems(EvansComputerMod.MODID);

    public static final DeferredItem<Item> HANDHELD_RADIO = ITEMS.registerItem("handheld_radio",
            props -> new HandheldRadioItem(props.stacksTo(1)));

    private RadioHandheldContent() {}

    public static void register(IEventBus modBus) {
        ITEMS.register(modBus);
        modBus.addListener((BuildCreativeModeTabContentsEvent e) -> {
            if (e.getTabKey() == ModCreativeTabs.TAB.getKey()) e.accept(HANDHELD_RADIO.get());
        });
        modBus.addListener(RadioHandheldContent::onPayloads);
        NeoForge.EVENT_BUS.addListener(HandheldServer::onPlayerTick);
        NeoForge.EVENT_BUS.addListener(HandheldServer::onLogout);
        if (FMLEnvironment.dist.isClient())
            com.example.evanscomputermod.radio.handheld.client.HandheldClientHooks.register();
    }

    private static void onPayloads(RegisterPayloadHandlersEvent e) {
        var r = e.registrar("1");
        r.playToClient(HandheldPackets.Audio.TYPE, HandheldPackets.Audio.STREAM_CODEC,
                (p, ctx) -> ctx.enqueueWork(() -> com.example.evanscomputermod.speaker.client.HandheldAudioClient.onAudio(p)));
        r.playToServer(HandheldPackets.Settings.TYPE, HandheldPackets.Settings.STREAM_CODEC, (p, ctx) -> ctx.enqueueWork(() -> {
            if (!(ctx.player() instanceof ServerPlayer player)) return;
            ItemStack stack = player.getItemInHand(p.mainHand() ? InteractionHand.MAIN_HAND : InteractionHand.OFF_HAND);
            if (!(stack.getItem() instanceof HandheldRadioItem)) return;
            HandheldBand band = HandheldBand.values()[Math.floorMod(p.band(), HandheldBand.values().length)];
            new HandheldSettings(p.on(), band, band.clamp(p.freqHz()), Math.max(0, Math.min(100, p.volume())),
                    Math.max(0, Math.min(100, p.squelch()))).write(stack);
        }));
    }
}
//?}
