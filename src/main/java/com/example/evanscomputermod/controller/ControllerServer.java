package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.ComputerRegistry;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import org.jetbrains.annotations.Nullable;

import java.util.Map;
import java.util.UUID;

/** Server side of the Wireless Controller: pairing, input and bindings. */
public final class ControllerServer {

    private ControllerServer() {}

    /** When each controller was last told "No signal" (throttle). */
    private static final java.util.Map<UUID, Long> lastNoSignal = new java.util.concurrent.ConcurrentHashMap<>();

    /** The controller with this id in the player's inventory (hands included), or empty. */
    public static ItemStack findController(Player player, UUID controllerId) {
        var inv = player.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (s.getItem() instanceof WirelessControllerItem && controllerId.equals(ControllerData.id(s))) {
                return s;
            }
        }
        return ItemStack.EMPTY;
    }

    /** The loaded Terminal running {@code computerId}, or null. */
    @Nullable
    private static TerminalBlockEntity findComputer(Level level, UUID computerId, @Nullable BlockPos pairedPos) {
        IComputerHost host = ComputerRegistry.get(computerId);
        if (host instanceof TerminalBlockEntity tbe && !tbe.isRemoved() && tbe.getLevel() == level) return tbe;
        // A computer that hasn't booted isn't registered: look where it was paired.
        if (pairedPos != null && level.isLoaded(pairedPos)
                && level.getBlockEntity(pairedPos) instanceof TerminalBlockEntity tbe
                && computerId.equals(tbe.getComputerId())) {
            return tbe;
        }
        return null;
    }

    /** Where a computer is, in world space (a computer on a Sable structure moves). */
    private static Vec3 worldPos(TerminalBlockEntity tbe) {
        Vec3 p = Vec3.atCenterOf(tbe.getBlockPos());
        //? if <=1.21.1 {
        p = com.example.evanscomputermod.sensor.SensorSable.toWorld(tbe.getLevel(), p);
        //?}
        return p;
    }

    /** "minecraft:overworld" etc. */
    public static String dimensionId(Level level) {
        //? if >=26.1 {
        return level.dimension().identifier().toString();
        //?} else
        /*return level.dimension().location().toString();*/
    }

    private static void status(ServerPlayer player, UUID controllerId, int slot, String message) {
        PacketDistributor.sendToPlayer(player, new ControllerPackets.Status(controllerId, slot, message));
    }

    public static void handleInput(ControllerPackets.Input packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (!(context.player() instanceof ServerPlayer player)) return;
            UUID id = packet.controllerId();
            ItemStack stack = findController(player, id);
            UUID computerId = stack.isEmpty() ? null : ControllerData.computer(stack);
            TerminalBlockEntity tbe = computerId == null ? null
                    : findComputer(player.level(), computerId, ControllerData.pairedPos(stack));

            if (!packet.connected()) {
                //? if <=1.21.1 {
                com.example.evanscomputermod.radio.controller.ControllerRadio.stop(id);
                //?}
                if (tbe != null && tbe.getControllers().disconnect(id)) status(player, id, 0, "Disconnected");
                return;
            }
            if (stack.isEmpty()) return;
            if (computerId == null) {
                status(player, id, 0, "Not paired: right-click a Terminal with it");
                return;
            }
            if (tbe == null) {
                status(player, id, 0, "Computer not found");
                return;
            }
            //? if <=1.21.1 {
            // The report goes over the 2.4 GHz radio medium to the computer's receiver module, which
            // applies it if it gets through (range = link budget: distance, walls, interference).
            var sent = com.example.evanscomputermod.radio.controller.ControllerRadio.send(player, id, packet.state(), tbe);
            if (sent == com.example.evanscomputermod.radio.controller.ControllerRadio.Result.NO_RECEIVER) {
                tbe.getControllers().disconnect(id);
                status(player, id, 0, "No receiver: install a Controller Receiver module");
                return;
            }
            if (sent == com.example.evanscomputermod.radio.controller.ControllerRadio.Result.SENT) {
                long since = com.example.evanscomputermod.radio.controller.ControllerRadio.sinceDeliveredMs(id, System.currentTimeMillis());
                if (since < 0 || since > 1000) {
                    long now = System.currentTimeMillis();
                    if (now - lastNoSignal.getOrDefault(id, 0L) > 1000) {
                        lastNoSignal.put(id, now);
                        status(player, id, tbe.getControllers().playerOf(id), "No signal");
                    }
                }
                return;
            }
            //?}
            WirelessControllerHub hub = tbe.getControllers();
            int before = hub.playerOf(id);
            int slot = hub.update(id, player.getUUID(), packet.state(), System.currentTimeMillis());
            if (slot == 0) {
                status(player, id, 0, "Computer already has " + WirelessControllerHub.MAX_CONTROLLERS + " controllers");
            } else if (slot != before) {
                status(player, id, slot, "Connected");
            }
        });
    }

    public static void handleBindings(ControllerPackets.Bindings packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            if (!(context.player() instanceof ServerPlayer player)) return;
            ItemStack stack = player.getItemInHand(packet.hand());
            if (!(stack.getItem() instanceof WirelessControllerItem)) return;
            Map<ControllerInput, String> merged = ControllerData.bindings(stack);
            merged.putAll(packet.bindings());
            ControllerData.setBindings(stack, merged);
        });
    }

    /** Pair a controller with the Terminal the player right-clicked. Server side. */
    public static void pair(ServerPlayer player, ItemStack stack, TerminalBlockEntity tbe) {
        UUID old = ControllerData.computer(stack);
        UUID oldId = ControllerData.id(stack);
        if (old != null && oldId != null && !old.equals(tbe.getComputerId())) {
            // Leave the old computer before switching.
            TerminalBlockEntity prev = findComputer(player.level(), old, ControllerData.pairedPos(stack));
            if (prev != null) prev.getControllers().disconnect(oldId);
        }
        BlockPos pos = tbe.getBlockPos();
        ControllerData.pair(stack, tbe.getComputerId(), pos, dimensionId(player.level()));
        player.sendSystemMessage(Component.translatable("message.evanscomputermod.controller.paired",
                pos.getX(), pos.getY(), pos.getZ()));
    }
}
