package com.example.evanscomputermod.client;

import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.client.gui.screens.Screen;
import net.neoforged.api.distmarker.Dist;
import net.neoforged.bus.api.EventPriority;
import net.neoforged.bus.api.SubscribeEvent;
import net.neoforged.fml.common.EventBusSubscriber;
import net.neoforged.neoforge.client.event.ScreenEvent;

/**
 * Gives ECM's computer screens ({@link ComputerInputScreen}) their input first.
 *
 * <p>NeoForge fires {@code ScreenEvent.*.Pre} before calling the screen's own
 * {@code mouseClicked}/{@code keyPressed}, and other mods (JEI's overlay, also
 * while it isn't drawn) consume clicks over their area there. Here, at the
 * highest priority, a press, release, drag or scroll over the screen's own
 * area (and the release and drags of a press it took, wherever they land),
 * and every key and typed character, is handed to the screen directly and the
 * event is cancelled, so nothing after us sees it. Clicks elsewhere on the
 * screen (beside the Terminal) are left to the usual handling.
 */
@EventBusSubscriber(modid = EvansComputerMod.MODID, value = Dist.CLIENT)
public final class ComputerScreenInput {
    /** Buttons whose press a computer screen took (their release and drags go to it too). */
    private static int pressed;
    /** Input events handed to computer screens (render checks). */
    private static volatile int dispatched;

    private ComputerScreenInput() {}

    public static int dispatched() {
        return dispatched;
    }

    private static ComputerInputScreen ours(Screen s) {
        return s instanceof ComputerInputScreen c ? c : null;
    }

    private static int bit(int button) {
        return button >= 0 && button < 31 ? 1 << button : 0;
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onMousePressed(ScreenEvent.MouseButtonPressed.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null || !s.ownsPointer(e.getMouseX(), e.getMouseY())) return;
        pressed |= bit(e.getButton());
        dispatched++;
        s.computerMousePressed(e.getMouseX(), e.getMouseY(), e.getButton());
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onMouseReleased(ScreenEvent.MouseButtonReleased.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null) return;
        int b = bit(e.getButton());
        if ((pressed & b) == 0 && !s.ownsPointer(e.getMouseX(), e.getMouseY())) return;
        pressed &= ~b;
        dispatched++;
        s.computerMouseReleased(e.getMouseX(), e.getMouseY(), e.getButton());
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onMouseDragged(ScreenEvent.MouseDragged.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null) return;
        if ((pressed & bit(e.getMouseButton())) == 0 && !s.ownsPointer(e.getMouseX(), e.getMouseY())) return;
        s.computerMouseDragged(e.getMouseX(), e.getMouseY(), e.getMouseButton(), e.getDragX(), e.getDragY());
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onMouseScrolled(ScreenEvent.MouseScrolled.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null || !s.ownsPointer(e.getMouseX(), e.getMouseY())) return;
        dispatched++;
        s.computerMouseScrolled(e.getMouseX(), e.getMouseY(), e.getScrollDeltaX(), e.getScrollDeltaY());
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onKeyPressed(ScreenEvent.KeyPressed.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null) return;
        if (!s.swallowsKey(e.getKeyCode())) {
            dispatched++;
            s.computerKeyPressed(e.getKeyCode(), e.getScanCode(), e.getModifiers());
        }
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onKeyReleased(ScreenEvent.KeyReleased.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null) return;
        if (!s.swallowsKey(e.getKeyCode())) s.computerKeyReleased(e.getKeyCode(), e.getScanCode(), e.getModifiers());
        e.setCanceled(true);
    }

    @SubscribeEvent(priority = EventPriority.HIGHEST)
    public static void onCharTyped(ScreenEvent.CharacterTyped.Pre e) {
        ComputerInputScreen s = ours(e.getScreen());
        if (s == null) return;
        int cp = e.getCodePoint();
        if (!s.swallowsChar(cp)) s.computerCharTyped(cp);
        e.setCanceled(true);
    }

    @SubscribeEvent
    public static void onClosing(ScreenEvent.Closing e) {
        pressed = 0;
    }
}
