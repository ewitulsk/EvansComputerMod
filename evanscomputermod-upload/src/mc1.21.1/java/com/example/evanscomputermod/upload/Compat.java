package com.example.evanscomputermod.upload;

import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;

/** Minecraft 1.21.1 / NeoForge 21.1 names of the calls that differ between versions. */
final class Compat {
    private Compat() {}

    static <T extends CustomPacketPayload> CustomPacketPayload.Type<T> type(String path) {
        return new CustomPacketPayload.Type<>(ResourceLocation.fromNamespaceAndPath(UploadSidecarMod.MODID, path));
    }

    static void sendToPlayer(ServerPlayer player, CustomPacketPayload payload) {
        PacketDistributor.sendToPlayer(player, payload);
    }


    static boolean hasChannel(ServerPlayer player, CustomPacketPayload.Type<?> type) {
        return player.connection != null && player.connection.hasChannel(type);
    }

    static boolean isOperator(ServerPlayer player) {
        return player.hasPermissions(2);
    }

}
