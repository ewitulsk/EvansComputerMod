package com.example.evanscomputermod.upload;

import com.example.evanscomputermod.api.upload.UploadBackends;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.common.Mod;
import net.neoforged.fml.config.ModConfig;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.registration.PayloadRegistrar;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

/**
 * Entry point of the file upload sidecar: players copy files from their real
 * computers into computers in the game. It installs the server side
 * ({@link ServerUploads}) as the main mod's upload backend, and registers the
 * upload payloads as optional, so a game or server without the sidecar still
 * connects (upload is then simply not offered). The player's side is
 * {@link ClientUploads}.
 */
@Mod(UploadSidecarMod.MODID)
public class UploadSidecarMod {
    public static final String MODID = "evanscomputermod_upload";
    private static final Logger LOGGER = LoggerFactory.getLogger(UploadSidecarMod.class);

    public UploadSidecarMod(IEventBus modEventBus, ModContainer container) {
        container.registerConfig(ModConfig.Type.SERVER, UploadConfig.SPEC);
        modEventBus.addListener(UploadSidecarMod::registerPayloads);
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) -> ServerUploads.INSTANCE.tick(e.getServer()));
        UploadBackends.install(ServerUploads.INSTANCE);
        LOGGER.info("File upload sidecar loaded: players can upload files to computers");
    }

    private static void registerPayloads(RegisterPayloadHandlersEvent event) {
        PayloadRegistrar r = event.registrar("1").optional();
        r.playToClient(UploadPackets.Pick.TYPE, UploadPackets.Pick.STREAM_CODEC,
                (m, ctx) -> ctx.enqueueWork(() -> ClientUploads.onPick(m)));
        r.playToClient(UploadPackets.Result.TYPE, UploadPackets.Result.STREAM_CODEC,
                (m, ctx) -> ctx.enqueueWork(() -> ClientUploads.onResult(m)));
        r.playToServer(UploadPackets.Begin.TYPE, UploadPackets.Begin.STREAM_CODEC, ServerUploads::onBegin);
        r.playToServer(UploadPackets.Chunk.TYPE, UploadPackets.Chunk.STREAM_CODEC, ServerUploads::onChunk);
        r.playToServer(UploadPackets.End.TYPE, UploadPackets.End.STREAM_CODEC, ServerUploads::onEnd);
        r.playToServer(UploadPackets.Cancel.TYPE, UploadPackets.Cancel.STREAM_CODEC, ServerUploads::onCancel);
    }
}
