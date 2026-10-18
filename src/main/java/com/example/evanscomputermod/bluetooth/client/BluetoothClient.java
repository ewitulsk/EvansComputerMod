package com.example.evanscomputermod.bluetooth.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.bluetooth.AirPodsItem;
import com.example.evanscomputermod.bluetooth.BluetoothPackets;
import com.example.evanscomputermod.bluetooth.BtFrame;
import com.example.evanscomputermod.speaker.client.EarbudsAudioClient;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.world.entity.EquipmentSlot;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RegisterKeyMappingsEvent;
import net.neoforged.neoforge.client.settings.KeyConflictContext;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.network.PacketDistributor;

/**
 * Client side of Bluetooth: stops the AirPods' audio when the phone stops sending, and the
 * "Tap AirPods" key (Controls, ePhone category, unbound by default). Pressing it twice
 * within {@link #DOUBLE_TAP_MS} is a double-tap on the earbuds while you wear them: the
 * AirPods tell their phone, which plays or pauses what it plays.
 */
public final class BluetoothClient {
    public static final KeyMapping TAP = new KeyMapping("key.evanscomputermod.airpods.tap", KeyConflictContext.IN_GAME,
            InputConstants.UNKNOWN, com.example.evanscomputermod.phone.client.PhoneClient.CATEGORY);
    /** Two presses this close together are a double-tap. */
    public static final long DOUBLE_TAP_MS = 400;

    private static long lastTapMs;

    private BluetoothClient() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterKeyMappingsEvent e) -> e.register(TAP));
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> {
            EarbudsAudioClient.tick();
            tick();
        });
    }

    private static void tick() {
        Minecraft mc = Minecraft.getInstance();
        while (TAP.consumeClick()) {
            if (mc.player == null || mc.screen != null || !(mc.player.getItemBySlot(EquipmentSlot.HEAD).getItem() instanceof AirPodsItem)) continue;
            long now = System.currentTimeMillis();
            if (now - lastTapMs <= DOUBLE_TAP_MS) {
                lastTapMs = 0;
                if (mc.getConnection() != null) PacketDistributor.sendToServer(new BluetoothPackets.Tap(BtFrame.GESTURE_DOUBLE_TAP));
            } else {
                lastTapMs = now;
            }
        }
    }
}
//?}
