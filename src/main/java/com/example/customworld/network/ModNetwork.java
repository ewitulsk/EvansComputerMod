package com.example.customworld.network;

import com.example.customworld.CustomWorldMod;
import net.minecraft.resources.ResourceLocation;
import net.minecraftforge.network.NetworkDirection;
import net.minecraftforge.network.NetworkRegistry;
import net.minecraftforge.network.simple.SimpleChannel;

/**
 * Handles registration of all network packets for the mod.
 */
public class ModNetwork {

    private static final String PROTOCOL_VERSION = "1.0.0";

    public static final SimpleChannel CHANNEL = NetworkRegistry.newSimpleChannel(
            new ResourceLocation(CustomWorldMod.MODID, "main"),
            () -> PROTOCOL_VERSION,
            PROTOCOL_VERSION::equals,
            PROTOCOL_VERSION::equals
    );

    private static int packetId = 0;

    public static void register() {
        // Client -> Server
        CHANNEL.messageBuilder(TerminalInputPacket.class, packetId++, NetworkDirection.PLAY_TO_SERVER)
                .encoder(TerminalInputPacket::encode)
                .decoder(TerminalInputPacket::decode)
                .consumerMainThread(TerminalInputPacket::handle)
                .add();

        // Server -> Client
        CHANNEL.messageBuilder(TerminalOutputPacket.class, packetId++, NetworkDirection.PLAY_TO_CLIENT)
                .encoder(TerminalOutputPacket::encode)
                .decoder(TerminalOutputPacket::decode)
                .consumerMainThread(TerminalOutputPacket::handle)
                .add();

        CustomWorldMod.LOGGER.info("Registered network packets");
    }
}
