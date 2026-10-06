package com.example.evanscomputermod.controller.client;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.client.GuiGfx;
import com.example.evanscomputermod.controller.ControllerData;
import com.example.evanscomputermod.controller.ControllerInput;
import com.example.evanscomputermod.controller.ControllerPackets;
import com.example.evanscomputermod.controller.ControllerState;
import com.example.evanscomputermod.controller.WirelessControllerItem;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.resources.Identifier;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RegisterGuiLayersEvent;
import net.neoforged.neoforge.client.gui.VanillaGuiLayers;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import org.jetbrains.annotations.Nullable;
import org.lwjgl.glfw.GLFW;

import java.util.EnumSet;
import java.util.HashMap;
import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.UUID;

/**
 * Client side of the Wireless Controller, modelled on Create's Linked
 * Controller: right-click to connect, and while connected the keys bound on the
 * item are read every tick (straight from the keyboard, so they work with the
 * Terminal GUI open too), turned into a {@link ControllerState} and sent to the
 * server. In the world, those keys stop doing their usual job (WASD no longer
 * walks) and a HUD shows the controller.
 *
 * <p>Two ways to be connected:
 * <ul>
 *   <li><b>In the world</b>: right-click with the controller in hand. Ends on
 *       right-click again, Escape (any screen opening), changing the held item,
 *       or the item leaving the hand.</li>
 *   <li><b>In the Terminal GUI</b>: the GUI's controller toggle, for a
 *       controller anywhere in the inventory that is paired with that computer.</li>
 * </ul>
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public final class ControllerClient {

    /** Resend the state this often (ticks) even if nothing changed: the server times out after 2 s. */
    private static final int KEEPALIVE_TICKS = 10;

    @Nullable private static UUID worldId;
    @Nullable private static InteractionHand worldHand;
    @Nullable private static UUID guiId;

    @Nullable private static UUID sentId;
    private static ControllerState lastSent = ControllerState.NEUTRAL;
    private static int ticksSinceSend;

    /** Latest status from the server for the connected controller. */
    private static int player;
    private static String statusMessage = "";
    private static ControllerState shown = ControllerState.NEUTRAL;

    private static final Map<String, InputConstants.Key> KEY_CACHE = new HashMap<>();

    private ControllerClient() {}

    // ------------------------------------------------------------ entry points

    /** Right-click with the controller: connect or disconnect in the world. */
    public static void toggle(InteractionHand hand) {
        Player p = Minecraft.getInstance().player;
        if (p == null) return;
        ItemStack stack = p.getItemInHand(hand);
        UUID id = ControllerData.id(stack);
        if (worldId != null && worldId.equals(id)) {
            stopWorld();
            return;
        }
        if (id == null || ControllerData.computer(stack) == null) {
            statusMessage = "Not paired: right-click a Terminal with it";
            player = 0;
            return;
        }
        stopGui();
        worldId = id;
        worldHand = hand;
        statusMessage = "Connecting...";
        player = 0;
        ticksSinceSend = KEEPALIVE_TICKS; // send right away
    }

    /** Sneak + right-click: the binding screen. */
    public static void openBindings(InteractionHand hand) {
        Minecraft.getInstance().setScreen(new ControllerBindingScreen(hand));
    }

    /** The Terminal GUI's toggle: drive {@code controllerId} from the GUI, or stop (null). */
    public static void setGuiController(@Nullable UUID controllerId) {
        if (controllerId == null) {
            stopGui();
            return;
        }
        stopWorld();
        guiId = controllerId;
        statusMessage = "Connecting...";
        player = 0;
        ticksSinceSend = KEEPALIVE_TICKS;
    }

    @Nullable
    public static UUID guiController() {
        return guiId;
    }

    public static boolean isWorldActive() {
        return worldId != null;
    }

    public static int player() {
        return player;
    }

    public static String statusMessage() {
        return statusMessage;
    }

    /** Server answer: connected as player N (0 = not connected) and why. */
    public static void onStatus(ControllerPackets.Status status) {
        if (!status.controllerId().equals(sentId) && !status.controllerId().equals(guiId)
                && !status.controllerId().equals(worldId)) {
            return;
        }
        player = status.player();
        statusMessage = status.message();
    }

    /** Whether {@code keyName} is bound on this controller (the Terminal GUI swallows those keys). */
    public static boolean isBound(ItemStack stack, InputConstants.Key key) {
        for (String name : ControllerData.bindings(stack).values()) {
            if (!name.isEmpty() && key.equals(key(name))) return true;
        }
        return false;
    }

    // ------------------------------------------------------------ ticking

    @SubscribeEvent
    public static void onClientTick(ClientTickEvent.Pre event) {
        Minecraft mc = Minecraft.getInstance();
        Player p = mc.player;
        if (p == null) {
            worldId = null;
            guiId = null;
            sentId = null;
            return;
        }

        ItemStack stack = ItemStack.EMPTY;
        UUID id = null;
        if (worldId != null) {
            ItemStack held = worldHand == null ? ItemStack.EMPTY : p.getItemInHand(worldHand);
            if (mc.screen != null || !(held.getItem() instanceof WirelessControllerItem)
                    || !worldId.equals(ControllerData.id(held))) {
                stopWorld();
            } else {
                stack = held;
                id = worldId;
            }
        } else if (guiId != null) {
            // GUI mode lives only as long as the Terminal GUI is open.
            stack = mc.screen instanceof com.example.evanscomputermod.block.TerminalScreen
                    ? findInInventory(p, guiId) : ItemStack.EMPTY;
            if (stack.isEmpty()) stopGui();
            else id = guiId;
        }

        if (id == null) {
            disconnectIfSent();
            return;
        }

        Map<ControllerInput, String> bindings = ControllerData.bindings(stack);
        Set<ControllerInput> pressed = EnumSet.noneOf(ControllerInput.class);
        Set<InputConstants.Key> boundKeys = new HashSet<>();
        long window = windowHandle(mc);
        for (Map.Entry<ControllerInput, String> e : bindings.entrySet()) {
            InputConstants.Key k = key(e.getValue());
            if (k == null) continue;
            boundKeys.add(k);
            if (isDown(window, k)) pressed.add(e.getKey());
        }
        if (worldId != null) suppress(mc, boundKeys);

        ControllerState state = ControllerState.of(pressed);
        shown = state;
        if (!id.equals(sentId) || !state.equals(lastSent) || ++ticksSinceSend >= KEEPALIVE_TICKS) {
            if (sentId != null && !id.equals(sentId)) send(sentId, false, ControllerState.NEUTRAL);
            send(id, true, state);
            sentId = id;
            lastSent = state;
            ticksSinceSend = 0;
        }
    }

    /** Keys the controller uses don't also move the player, open the inventory, etc. */
    private static void suppress(Minecraft mc, Set<InputConstants.Key> boundKeys) {
        for (KeyMapping km : mc.options.keyMappings) {
            if (boundKeys.contains(km.getKey())) {
                km.setDown(false);
                while (km.consumeClick()) {
                    // drain queued presses
                }
            }
        }
    }

    private static void stopWorld() {
        worldId = null;
        worldHand = null;
        disconnectIfSent();
    }

    private static void stopGui() {
        guiId = null;
        disconnectIfSent();
    }

    private static void disconnectIfSent() {
        if (sentId != null) {
            send(sentId, false, ControllerState.NEUTRAL);
            sentId = null;
            lastSent = ControllerState.NEUTRAL;
            shown = ControllerState.NEUTRAL;
            player = 0;
        }
    }

    private static void send(UUID id, boolean connected, ControllerState state) {
        if (Minecraft.getInstance().getConnection() == null) return;
        ClientPacketDistributor.sendToServer(new ControllerPackets.Input(id, connected, state));
    }

    private static ItemStack findInInventory(Player p, UUID id) {
        var inv = p.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) {
            ItemStack s = inv.getItem(i);
            if (s.getItem() instanceof WirelessControllerItem && id.equals(ControllerData.id(s))) return s;
        }
        return ItemStack.EMPTY;
    }

    // ------------------------------------------------------------ keys

    /** The key for a binding name, or null if unbound or unknown. */
    @Nullable
    public static InputConstants.Key key(String name) {
        if (name == null || name.isEmpty()) return null;
        return KEY_CACHE.computeIfAbsent(name, n -> {
            try {
                InputConstants.Key k = InputConstants.getKey(n);
                return k == InputConstants.UNKNOWN ? null : k;
            } catch (RuntimeException e) {
                return null;
            }
        });
    }

    /** Short label for a binding ("J", "Up", "Num 8", "-" for unbound). */
    public static String keyLabel(String name) {
        InputConstants.Key k = key(name);
        if (k == null) return "-";
        return k.getDisplayName().getString();
    }

    private static boolean isDown(long window, InputConstants.Key k) {
        int code = k.getValue();
        if (k.getType() == InputConstants.Type.KEYSYM) {
            return code >= 0 && GLFW.glfwGetKey(window, code) == GLFW.GLFW_PRESS;
        }
        if (k.getType() == InputConstants.Type.MOUSE) {
            return code >= 0 && GLFW.glfwGetMouseButton(window, code) == GLFW.GLFW_PRESS;
        }
        return false;
    }

    private static long windowHandle(Minecraft mc) {
        //? if >=26.1 {
        return mc.getWindow().handle();
        //?} else
        /*return mc.getWindow().getWindow();*/
    }

    // ------------------------------------------------------------ HUD

    @SubscribeEvent
    public static void registerLayers(RegisterGuiLayersEvent event) {
        event.registerAbove(VanillaGuiLayers.HOTBAR,
                Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "wireless_controller"),
                (g, delta) -> renderHud(new GuiGfx(g)));
    }

    private static void renderHud(GuiGfx g) {
        if (worldId == null) return;
        Minecraft mc = Minecraft.getInstance();
        int w = mc.getWindow().getGuiScaledWidth();
        int h = mc.getWindow().getGuiScaledHeight();
        String title = player > 0 ? "Controller " + player : "Controller";
        String line = statusMessage;
        int cx = w / 2;
        int y = h - 92;
        int width = Math.max(150, Math.max(mc.font.width(title), mc.font.width(line)) + 16);
        g.box(cx - width / 2, y, cx + width / 2, y + 44, 0xC0101018, player > 0 ? 0xFF3FB950 : 0xFF8B949E);
        g.centeredText(mc.font, title, cx, y + 4, 0xFFFFFFFF);
        g.centeredText(mc.font, line, cx, y + 14, 0xFFB0B0B0);
        // Buttons held, as small lit cells.
        ControllerInput.Button[] buttons = ControllerInput.Button.values();
        int cell = 8;
        int x0 = cx - buttons.length * cell / 2;
        for (int i = 0; i < buttons.length; i++) {
            boolean down = shown.isDown(buttons[i]);
            g.fill(x0 + i * cell + 1, y + 27, x0 + i * cell + cell - 1, y + 35, down ? 0xFF3FB950 : 0xFF30363D);
        }
        // Sticks as dots.
        stick(g, cx - width / 2 + 12, y + 31, shown.lx(), shown.ly());
        stick(g, cx + width / 2 - 12, y + 31, shown.rx(), shown.ry());
    }

    private static void stick(GuiGfx g, int cx, int cy, int x, int y) {
        g.fill(cx - 6, cy - 6, cx + 6, cy + 6, 0xFF30363D);
        int dx = x * 4 / 127;
        int dy = -y * 4 / 127;
        g.fill(cx + dx - 2, cy + dy - 2, cx + dx + 2, cy + dy + 2, 0xFFE6EDF3);
    }

    /** Forget all state (world change). */
    static void reset() {
        worldId = null;
        guiId = null;
        sentId = null;
        player = 0;
        statusMessage = "";
    }
}
