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
import net.neoforged.bus.api.EventPriority;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.InputEvent;
import net.neoforged.neoforge.client.event.ScreenEvent;
import net.neoforged.neoforge.client.event.RegisterGuiLayersEvent;
import net.neoforged.neoforge.client.gui.VanillaGuiLayers;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import org.jetbrains.annotations.Nullable;
import org.lwjgl.glfw.GLFW;

import java.util.EnumSet;
import java.util.HashMap;
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
 *       right-click again, Escape (which then doesn't open the pause menu), any screen opening, changing the held item,
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

    /**
     * Escape in the world (no screen open): disconnect a world-connected
     * controller. Returns true if it did, and the key should go no further
     * (no pause menu). Called from ControllerEscapeMixin on 1.21.1.
     */
    public static boolean disconnectOnEscape() {
        if (worldId == null) return false;
        stopWorld();
        return true;
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
        long window = windowHandle(mc);
        // In the Terminal GUI the keys only drive the controller while a program
        // shows graphics there; otherwise they type, and the controller stays
        // connected but idle.
        boolean readKeys = worldId != null
                || (mc.screen instanceof com.example.evanscomputermod.block.TerminalScreen ts
                        && ts.controllerKeysActive());
        for (Map.Entry<ControllerInput, String> e : bindings.entrySet()) {
            InputConstants.Key k = key(e.getValue());
            if (k == null) continue;
            if (readKeys && isDown(window, k)) pressed.add(e.getKey());
        }
        if (worldId != null) suppressKeyboard(mc);

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

    // ------------------------------------------------------------ key priority
    //
    // The controller's keys must not reach other mods. In a screen, mods such
    // as JEI act on NeoForge's screen key events, which fire before the
    // screen's own keyPressed: cancel them first (highest priority). In the
    // world, mods read their key mappings, which vanilla clicks before
    // InputEvent.Key fires: unpress and drain those mappings straight away,
    // ahead of other listeners and the next tick.

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onScreenKeyPressed(ScreenEvent.KeyPressed.Pre event) {
        if (event.getScreen() instanceof com.example.evanscomputermod.block.TerminalScreen ts
                && ts.swallowsControllerKey(event.getKeyCode())) {
            event.setCanceled(true);
        }
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onScreenKeyReleased(ScreenEvent.KeyReleased.Pre event) {
        if (event.getScreen() instanceof com.example.evanscomputermod.block.TerminalScreen ts
                && ts.swallowsControllerKey(event.getKeyCode())) {
            event.setCanceled(true);
        }
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onScreenCharTyped(ScreenEvent.CharacterTyped.Pre event) {
        if (event.getScreen() instanceof com.example.evanscomputermod.block.TerminalScreen ts
                && ts.swallowsControllerChar((char) event.getCodePoint())) {
            event.setCanceled(true);
        }
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onKey(InputEvent.Key event) {
        if (worldId == null || worldHand == null) return;
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null || mc.screen != null) return;
        suppressKeyboard(mc);
    }

    /**
     * While connected in the world the keyboard belongs to the controller:
     * no keyboard key mapping (vanilla or another mod's) fires, bound to the
     * controller or not, so L doesn't open advancements and WASD doesn't walk.
     * Mouse mappings stay (right-click disconnects), as do Escape (not a
     * mapping), screenshots and fullscreen.
     */
    private static void suppressKeyboard(Minecraft mc) {
        for (KeyMapping km : mc.options.keyMappings) {
            if (km == mc.options.keyScreenshot || km == mc.options.keyFullscreen) continue;
            if (km.getKey().getType() != InputConstants.Type.KEYSYM
                    && km.getKey().getType() != InputConstants.Type.SCANCODE) continue;
            km.setDown(false);
            while (km.consumeClick()) {
                // drain queued presses
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

    /**
     * A small panel at the right edge of the screen: player number (or the
     * status while not connected), both sticks and a lit cell per held button.
     */
    private static void renderHud(GuiGfx g) {
        if (worldId == null) return;
        Minecraft mc = Minecraft.getInstance();
        int w = mc.getWindow().getGuiScaledWidth();
        int h = mc.getWindow().getGuiScaledHeight();

        ControllerInput.Button[] buttons = ControllerInput.Button.values();
        int cols = (buttons.length + 1) / 2;
        int cell = 3;
        int stick = 7;
        int pad = 2;
        int innerW = stick + pad + cols * cell + pad + stick;
        int width = innerW + pad * 2;
        int height = pad + mc.font.lineHeight + pad + stick + pad;
        int x0 = w - width - 4;
        int y0 = h / 2 - height / 2;
        int border = player > 0 ? 0xFF3FB950 : 0xFF8B949E;
        g.box(x0, y0, x0 + width, y0 + height, 0xA0101018, border);

        String label = player > 0 ? "P" + player : statusMessage;
        int labelW = mc.font.width(label);
        // Right-align, so a long status runs off to the left, never off screen.
        g.text(mc.font, label, player > 0 ? x0 + pad + 1 : x0 + width - pad - labelW,
                y0 + pad, player > 0 ? 0xFFE6EDF3 : 0xFFB0B0B0);

        int sy = y0 + pad + mc.font.lineHeight + pad;
        int lx = x0 + pad;
        stick(g, lx, sy, stick, shown.lx(), shown.ly());
        int bx = lx + stick + pad;
        for (int i = 0; i < buttons.length; i++) {
            int cx = bx + (i % cols) * cell;
            int cy = sy + (i / cols) * (cell + 1);
            g.fill(cx, cy, cx + cell - 1, cy + cell, shown.isDown(buttons[i]) ? 0xFF3FB950 : 0xFF30363D);
        }
        stick(g, bx + cols * cell + pad, sy, stick, shown.rx(), shown.ry());
    }

    private static void stick(GuiGfx g, int x, int y, int size, int sx, int sy) {
        g.fill(x, y, x + size, y + size, 0xFF30363D);
        int c = size / 2;
        int dx = sx * (c - 1) / 127;
        int dy = -sy * (c - 1) / 127;
        g.fill(x + c + dx - 1, y + c + dy - 1, x + c + dx + 1, y + c + dy + 1, 0xFFE6EDF3);
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
