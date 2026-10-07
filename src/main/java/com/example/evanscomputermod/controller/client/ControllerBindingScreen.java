package com.example.evanscomputermod.controller.client;

import com.example.evanscomputermod.client.GuiGfx;
import com.example.evanscomputermod.controller.ControllerData;
import com.example.evanscomputermod.controller.ControllerInput;
import com.example.evanscomputermod.controller.ControllerPackets;
import com.example.evanscomputermod.controller.WirelessControllerItem;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.screens.Screen;
//? if >=26.1 {
import net.minecraft.client.input.KeyEvent;
import net.minecraft.client.input.MouseButtonEvent;
//?}
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;
import org.jetbrains.annotations.Nullable;
import org.lwjgl.glfw.GLFW;

import java.util.EnumMap;
import java.util.Map;

/**
 * Key bindings of the Wireless Controller in hand (sneak + right-click), in the
 * spirit of Create's Linked Controller screen: a picture of the controller with
 * every button and stick direction on it, each showing its key. Click one, then
 * press the key for it (Escape cancels, Delete unbinds). "Defaults" restores the
 * default layout. The bindings are saved to the item when the screen closes.
 */
public class ControllerBindingScreen extends Screen {

    private static final int BODY = 0xFF2B2F36;
    private static final int BODY_EDGE = 0xFF15171B;
    private static final int CELL = 0xFF3A3F47;
    private static final int CELL_HOVER = 0xFF4E5560;
    private static final int CELL_EDGE = 0xFF0D0F12;
    private static final int LISTENING = 0xFF9E6A03;
    private static final int TEXT = 0xFFE6EDF3;
    private static final int DIM = 0xFF8B949E;

    private static final int CW = 34;
    private static final int CH = 14;

    /** Where each input sits, relative to the centre of the controller. */
    private record Spot(ControllerInput input, int dx, int dy, String tag) {}

    private static final Spot[] SPOTS = {
            new Spot(ControllerInput.LT, -118, -104, "LT"),
            new Spot(ControllerInput.LB, -118, -80, "LB"),
            new Spot(ControllerInput.RT, 118, -104, "RT"),
            new Spot(ControllerInput.RB, 118, -80, "RB"),
            new Spot(ControllerInput.BACK, -40, -40, "Back"),
            new Spot(ControllerInput.GUIDE, 0, -54, "Guide"),
            new Spot(ControllerInput.START, 40, -40, "Start"),
            new Spot(ControllerInput.LS_UP, -100, -40, "LS ^"),
            new Spot(ControllerInput.LS_LEFT, -138, -20, "LS <"),
            new Spot(ControllerInput.LS, -100, -20, "LS"),
            new Spot(ControllerInput.LS_RIGHT, -62, -20, "LS >"),
            new Spot(ControllerInput.LS_DOWN, -100, 0, "LS v"),
            new Spot(ControllerInput.Y, 100, -40, "Y"),
            new Spot(ControllerInput.X, 62, -20, "X"),
            new Spot(ControllerInput.B, 138, -20, "B"),
            new Spot(ControllerInput.A, 100, 0, "A"),
            new Spot(ControllerInput.DPAD_UP, -60, 22, "D ^"),
            new Spot(ControllerInput.DPAD_LEFT, -98, 42, "D <"),
            new Spot(ControllerInput.DPAD_RIGHT, -22, 42, "D >"),
            new Spot(ControllerInput.DPAD_DOWN, -60, 62, "D v"),
            new Spot(ControllerInput.RS_UP, 60, 22, "RS ^"),
            new Spot(ControllerInput.RS_LEFT, 22, 42, "RS <"),
            new Spot(ControllerInput.RS, 60, 42, "RS"),
            new Spot(ControllerInput.RS_RIGHT, 98, 42, "RS >"),
            new Spot(ControllerInput.RS_DOWN, 60, 62, "RS v"),
    };

    private final InteractionHand hand;
    private final Map<ControllerInput, String> bindings = new EnumMap<>(ControllerInput.class);
    private final Map<ControllerInput, String> original;
    @Nullable private ControllerInput listening;

    public ControllerBindingScreen(InteractionHand hand) {
        super(Component.translatable("screen.evanscomputermod.controller_bindings"));
        this.hand = hand;
        ItemStack stack = heldStack();
        Map<ControllerInput, String> current = stack.isEmpty()
                ? ControllerData.defaultBindings() : ControllerData.bindings(stack);
        bindings.putAll(current);
        original = new EnumMap<>(current);
    }

    private ItemStack heldStack() {
        var p = Minecraft.getInstance().player;
        if (p == null) return ItemStack.EMPTY;
        ItemStack s = p.getItemInHand(hand);
        return s.getItem() instanceof WirelessControllerItem ? s : ItemStack.EMPTY;
    }

    private int cx() { return width / 2; }

    private int cy() { return height / 2 + 6; }

    private int[] rect(Spot s) {
        int x = cx() + s.dx() - CW / 2;
        int y = cy() + s.dy() - CH / 2;
        return new int[]{x, y, x + CW, y + CH};
    }

    private int[] defaultsButton() {
        return new int[]{cx() - 104, cy() + 92, cx() - 4, cy() + 108};
    }

    private int[] doneButton() {
        return new int[]{cx() + 4, cy() + 92, cx() + 104, cy() + 108};
    }

    private static boolean inside(int[] r, double x, double y) {
        return x >= r[0] && x < r[2] && y >= r[1] && y < r[3];
    }

    // ------------------------------------------------------------ rendering

    //? if >=26.1 {
    @Override
    public void extractRenderState(net.minecraft.client.gui.GuiGraphicsExtractor g, int mouseX, int mouseY, float partialTick) {
        draw(new GuiGfx(g), mouseX, mouseY);
    }
    //?} else {
    /*@Override
    public void render(net.minecraft.client.gui.GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        super.render(g, mouseX, mouseY, partialTick);
        draw(new GuiGfx(g), mouseX, mouseY);
    }*/
    //?}

    private void draw(GuiGfx g, int mouseX, int mouseY) {
        var font = Minecraft.getInstance().font;
        g.fill(0, 0, width, height, 0xC0000000);
        int cx = cx();
        int cy = cy();

        g.centeredText(font, title.getString(), cx, cy - 126, TEXT);

        // Body: a wide rounded block with two grips.
        g.box(cx - 160, cy - 66, cx + 160, cy + 16, BODY, BODY_EDGE);
        g.box(cx - 160, cy + 14, cx - 70, cy + 82, BODY, BODY_EDGE);
        g.box(cx + 70, cy + 14, cx + 160, cy + 82, BODY, BODY_EDGE);
        g.fill(cx - 158, cy + 12, cx - 72, cy + 18, BODY);
        g.fill(cx + 72, cy + 12, cx + 158, cy + 18, BODY);
        // Shoulders.
        g.box(cx - 150, cy - 108, cx - 86, cy - 66, 0xFF22252B, BODY_EDGE);
        g.box(cx + 86, cy - 108, cx + 150, cy - 66, 0xFF22252B, BODY_EDGE);

        String hover = null;
        for (Spot s : SPOTS) {
            int[] r = rect(s);
            boolean over = inside(r, mouseX, mouseY);
            int fill = s.input() == listening ? LISTENING : over ? CELL_HOVER : CELL;
            g.box(r[0], r[1], r[2], r[3], fill, CELL_EDGE);
            String key = ControllerClient.keyLabel(bindings.get(s.input()));
            if (font.width(key) > CW - 4) key = font.plainSubstrByWidth(key, CW - 6) + ".";
            g.centeredText(font, key, (r[0] + r[2]) / 2, r[1] + 3, key.equals("-") ? DIM : TEXT);
            g.centeredText(font, s.tag(), (r[0] + r[2]) / 2, r[3] + 1, DIM);
            if (over) hover = s.input().label() + ": " + ControllerClient.keyLabel(bindings.get(s.input()));
        }

        String help;
        if (listening != null) {
            help = "Press a key for " + listening.label() + "  (Esc: cancel, Delete: unbind)";
        } else if (hover != null) {
            help = hover;
        } else {
            help = "Click a button, then press the key for it";
        }
        g.centeredText(font, help, cx, cy + 84, listening != null ? 0xFFF0B429 : DIM);

        button(g, font, defaultsButton(), "Defaults", mouseX, mouseY);
        button(g, font, doneButton(), "Done", mouseX, mouseY);
    }

    private void button(GuiGfx g, net.minecraft.client.gui.Font font, int[] r, String label, int mx, int my) {
        g.box(r[0], r[1], r[2], r[3], inside(r, mx, my) ? CELL_HOVER : CELL, CELL_EDGE);
        g.centeredText(font, label, (r[0] + r[2]) / 2, r[1] + 4, TEXT);
    }

    // ------------------------------------------------------------ input

    //? if >=26.1 {
    @Override
    public boolean mouseClicked(MouseButtonEvent event, boolean doubleClick) {
        if (click(event.x(), event.y(), event.button())) return true;
        return super.mouseClicked(event, doubleClick);
    }

    @Override
    public boolean keyPressed(KeyEvent event) {
        if (listening != null) {
            bindKey(event.key(), InputConstants.getKey(event));
            return true;
        }
        return super.keyPressed(event);
    }
    //?} else {
    /*@Override
    public boolean mouseClicked(double mouseX, double mouseY, int button) {
        if (click(mouseX, mouseY, button)) return true;
        return super.mouseClicked(mouseX, mouseY, button);
    }

    @Override
    public boolean keyPressed(int keyCode, int scanCode, int modifiers) {
        if (listening != null) {
            bindKey(keyCode, InputConstants.getKey(keyCode, scanCode));
            return true;
        }
        return super.keyPressed(keyCode, scanCode, modifiers);
    }*/
    //?}

    private boolean click(double x, double y, int button) {
        if (button != 0) return false;
        if (inside(defaultsButton(), x, y)) {
            bindings.putAll(ControllerData.defaultBindings());
            listening = null;
            return true;
        }
        if (inside(doneButton(), x, y)) {
            onClose();
            return true;
        }
        for (Spot s : SPOTS) {
            if (inside(rect(s), x, y)) {
                listening = s.input() == listening ? null : s.input();
                return true;
            }
        }
        listening = null;
        return false;
    }

    private void bindKey(int glfwKey, InputConstants.Key key) {
        ControllerInput target = listening;
        listening = null;
        if (target == null || glfwKey == GLFW.GLFW_KEY_ESCAPE) return;
        if (glfwKey == GLFW.GLFW_KEY_DELETE) {
            bindings.put(target, "");
            return;
        }
        String name = key.getName();
        if (!ControllerData.isValidKeyName(name)) return;
        // One key, one input: take it off anything else that had it.
        for (Map.Entry<ControllerInput, String> e : bindings.entrySet()) {
            if (e.getValue().equals(name)) e.setValue("");
        }
        bindings.put(target, name);
    }

    @Override
    public void onClose() {
        if (!bindings.equals(original) && !heldStack().isEmpty()) {
            ClientPacketDistributor.sendToServer(new ControllerPackets.Bindings(hand, new EnumMap<>(bindings)));
        }
        super.onClose();
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }
}
