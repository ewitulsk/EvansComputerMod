package com.example.evanscomputermod.upload;

import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.neoforged.neoforge.network.PacketDistributor;

/** Client-only calls whose names differ between Minecraft versions (1.21.1). */
final class ClientCompat {
    private ClientCompat() {}

    static void sendToServer(CustomPacketPayload payload) {
        PacketDistributor.sendToServer(payload);
    }

    /** A line in the player's chat. */
    static void clientMessage(String text) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.gui != null) mc.gui.getChat().addMessage(Component.literal(text));
    }
}
