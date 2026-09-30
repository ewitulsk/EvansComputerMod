package com.example.evanscomputermod.sensor.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.sensor.SensorContent;
import com.example.evanscomputermod.sensor.wire.SensorWireItem;
import com.example.evanscomputermod.sensor.wire.WirePackets;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.EntityRenderersEvent;
import net.neoforged.neoforge.client.event.ModelEvent;
import net.neoforged.neoforge.client.event.RegisterKeyMappingsEvent;
import net.neoforged.neoforge.client.event.RenderLevelStageEvent;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.entity.player.PlayerInteractEvent;
import net.neoforged.neoforge.network.PacketDistributor;
import org.lwjgl.glfw.GLFW;

/** Client setup for sensors and Sensor Wire. */
public final class SensorClient {
    /** Hold while placing Sensor Wire for straight L/Z paths instead of surface routing. */
    public static final KeyMapping ALTERNATE_PLACEMENT = new KeyMapping("key.evanscomputermod.alternate_wire_placement",
            InputConstants.Type.KEYSYM, GLFW.GLFW_KEY_LEFT_ALT, "key.categories.evanscomputermod");

    private static boolean alternateSent;

    private SensorClient() {
    }

    public static void register(IEventBus modBus) {
        modBus.addListener((EntityRenderersEvent.RegisterRenderers e) -> {
            e.registerEntityRenderer(SensorContent.BLOCK_WIRE.get(), BlockWireRenderer::new);
            e.registerBlockEntityRenderer(SensorContent.LIDAR_SENSOR_BE.get(), LidarSensorRenderer::new);
        });
        modBus.addListener((ModelEvent.RegisterAdditional e) -> e.register(LidarSensorRenderer.HEAD));
        modBus.addListener((RegisterKeyMappingsEvent e) -> e.register(ALTERNATE_PLACEMENT));
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> tick());
        NeoForge.EVENT_BUS.addListener(WirePreview::render);
        NeoForge.EVENT_BUS.addListener((PlayerInteractEvent.RightClickEmpty e) -> {
            if(e.getEntity().isShiftKeyDown() && e.getItemStack().is(SensorContent.WIRE_CUTTERS))
                ClientWireInteractions.cutClear();
        });
    }

    public static boolean alternatePlacementHeld() {
        return ALTERNATE_PLACEMENT.isDown();
    }

    private static void tick() {
        var mc = Minecraft.getInstance();
        if(mc.player == null || mc.level == null)
            return;
        WirePreview.tick();
        ClientWireInteractions.clientTick();
        // Tell the server while the alternate-placement key is held with a wire in hand.
        boolean held = alternatePlacementHeld() && mc.player.getMainHandItem().getItem() instanceof SensorWireItem;
        if(held != alternateSent && mc.getConnection() != null) {
            alternateSent = held;
            PacketDistributor.sendToServer(new WirePackets.Alternate(held));
        }
    }
}
//?}
