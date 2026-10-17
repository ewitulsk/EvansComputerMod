package com.example.evanscomputermod.card.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardSecretGuard;
import net.minecraft.network.protocol.game.ServerboundSetCreativeModeSlotPacket;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.server.network.ServerGamePacketListenerImpl;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.Shadow;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * A creative client sends whole item stacks back; SIM keys in them are only
 * placeholders (clients never get the key). Put the server's key back before
 * the stack is applied (see CardSecretGuard).
 */
@Mixin(ServerGamePacketListenerImpl.class)
public abstract class CreativeSlotMixin {
    @Shadow
    public ServerPlayer player;

    @Inject(method = "handleSetCreativeModeSlot", at = @At(value = "INVOKE",
            target = "Lnet/minecraft/network/protocol/PacketUtils;ensureRunningOnSameThread(Lnet/minecraft/network/protocol/Packet;Lnet/minecraft/network/PacketListener;Lnet/minecraft/server/level/ServerLevel;)V",
            shift = At.Shift.AFTER))
    private void evanscomputermod$restoreSimKeys(ServerboundSetCreativeModeSlotPacket packet, CallbackInfo ci) {
        if (player.gameMode.isCreative()) CardSecretGuard.beforeCreativeSet(player, packet.slotNum(), packet.itemStack());
    }
}
//?}
