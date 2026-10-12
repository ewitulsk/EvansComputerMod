package com.example.evanscomputermod.phone.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.computer.display.PanelTier;
import com.example.evanscomputermod.network.ClientDisplayCache;
import com.example.evanscomputermod.network.MouseInputPacket;
import com.example.evanscomputermod.network.TerminalInputPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.phone.PhoneItemData;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.resources.ResourceLocation;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.network.PacketDistributor;
import org.jetbrains.annotations.Nullable;
import org.lwjgl.glfw.GLFW;

import java.util.UUID;

/**
 * The phone opened full screen (right-click with it in hand). Every key goes
 * to the phone and the mouse is a finger: typed text and keys travel as
 * {@code TerminalInputPacket}s (VT sequences for arrows, Enter, Backspace,
 * Ctrl+letter), the mouse as {@code MouseInputPacket}s on display 0 in the
 * phone's pixels (graphics pixels while a program shows graphics). Both are
 * addressed by the phone's UUID; the server takes them only from the holder.
 * The compositor turns these into UI events. Escape closes the phone.
 */
public class PhoneScreen extends Screen {
    private final InteractionHand hand;
    @Nullable private UUID phone;
    private int sx, sy, sw, sh;
    private byte buttons;
    private int lastX = -1, lastY = -1;
    private int keysSent;

    public PhoneScreen(InteractionHand hand) {
        super(Component.translatable("item.evanscomputermod.ephone"));
        this.hand = hand;
    }

    public static void open(InteractionHand hand) {
        Minecraft.getInstance().setScreen(new PhoneScreen(hand));
    }

    @Nullable
    private ItemStack stack() {
        var p = Minecraft.getInstance().player;
        return p == null ? null : p.getItemInHand(hand);
    }

    @Override
    protected void init() {
        ItemStack s = stack();
        phone = s == null ? null : PhoneItemData.id(s);
        layout();
        if (phone != null) PacketDistributor.sendToServer(new TerminalReadyPacket(phone, (byte) IComputerHost.DISPLAY_OWN, 0));
    }

    /** The screen rectangle: the panel's aspect, 85 % of the window height. */
    private void layout() {
        PanelTier tier = tier();
        int[] size = PhoneScreenTexture.size(tier != null ? tier : PanelTier.COLOR_LCD);
        sh = (int) (height * 0.85);
        sw = sh * size[0] / size[1];
        sx = (width - sw) / 2;
        sy = (height - sh) / 2;
    }

    @Nullable
    private PanelTier tier() {
        ItemStack s = stack();
        return s == null ? null : PhoneHardware.of(s).tier();
    }

    @Override
    public void tick() {
        ItemStack s = stack();
        UUID now = s == null ? null : PhoneItemData.id(s);
        if (phone == null && now != null) init();   // first use: the server just gave the phone its id
        else if (phone != null && !phone.equals(now)) onClose();   // the phone left the hand
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        g.fill(0, 0, width, height, 0xC0000000);
        int bezel = Math.max(4, sw / 14);
        g.fill(sx - bezel, sy - bezel * 2, sx + sw + bezel, sy + sh + bezel * 2, 0xFF1B1F24);
        g.fill(sx - bezel + 1, sy - bezel * 2 + 1, sx + sw + bezel - 1, sy + sh + bezel * 2 - 1, 0xFF2D333B);
        g.fill(sx, sy, sx + sw, sy + sh, 0xFF05080B);
        g.fill(sx + sw / 2 - bezel, sy - bezel - 2, sx + sw / 2 + bezel, sy - bezel + 1, 0xFF0D1117);
        PanelTier tier = tier();
        ItemStack s = stack();
        if (phone == null || s == null) {
            g.drawCenteredString(font, Component.translatable("gui.evanscomputermod.ephone.starting"), width / 2, height / 2, 0xFF8B949E);
            return;
        }
        if (tier == null) {
            g.drawCenteredString(font, Component.translatable("gui.evanscomputermod.no_panel"), width / 2, height / 2, 0xFF777777);
            return;
        }
        PhoneClient.wantFrames(phone, IComputerHost.DISPLAY_OWN);
        ResourceLocation tex = PhoneScreenTexture.get(phone, IComputerHost.DISPLAY_OWN, tier, PhoneHardware.of(s).tone());
        if (tex == null) {
            g.drawCenteredString(font, Component.translatable("gui.evanscomputermod.ephone.starting"), width / 2, height / 2, 0xFF8B949E);
            return;
        }
        int[] size = PhoneScreenTexture.size(tier);
        g.blit(tex, sx, sy, sw, sh, 0, 0, size[0], size[1], size[0], size[1]);
        PhoneClient.Status st = PhoneClient.status(phone);
        if (st != null) {
            String pct = String.format(java.util.Locale.ROOT, "%.0f%%", st.battery());
            g.drawString(font, pct, sx + sw - font.width(pct), sy + sh + bezel / 2, 0xFFB0B0B0, false);
        }
    }

    @Override
    public void renderBackground(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        // drawn in render
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }

    // ------------------------------------------------------------ keyboard: every key goes to the phone

    @Override
    public boolean keyPressed(int key, int scan, int mods) {
        if (key == GLFW.GLFW_KEY_ESCAPE) {
            onClose();
            return true;
        }
        rawKey(key, mods, true);
        String seq = encodeKey(key, mods);
        if (seq != null) type(seq);
        return true;
    }

    @Override
    public boolean keyReleased(int key, int scan, int mods) {
        rawKey(key, mods, false);
        return true;
    }

    /** GLFW keys held, to tell repeats from presses. */
    private final java.util.Set<Integer> held = new java.util.HashSet<>();

    /**
     * The raw key for the phone's GUI (UI_PROTOCOL §7): a USB HID usage with the
     * modifier bits, next to the console string; the server uses whichever
     * matches what the phone shows (the GUI, or a legacy program's window).
     */
    private void rawKey(int key, int mods, boolean down) {
        if (phone == null) return;
        int hid = com.example.evanscomputermod.computer.ui.HidKeys.fromGlfw(key);
        byte action;
        if (down) {
            action = held.add(key) ? com.example.evanscomputermod.network.KeyInputPacket.DOWN
                    : com.example.evanscomputermod.network.KeyInputPacket.REPEAT;
        } else {
            if (!held.remove(key)) return;
            action = com.example.evanscomputermod.network.KeyInputPacket.UP;
        }
        if (hid == 0) return;
        keysSent++;
        PacketDistributor.sendToServer(com.example.evanscomputermod.network.KeyInputPacket.key(phone,
                (byte) IComputerHost.DISPLAY_OWN, action, hid, mods & com.example.evanscomputermod.computer.ui.HidKeys.MODS_MASK));
    }

    /** VT sequence or control character for a key that doesn't produce typed text, or null. */
    @Nullable
    static String encodeKey(int key, int mods) {
        boolean ctrl = (mods & GLFW.GLFW_MOD_CONTROL) != 0;
        if (ctrl && key >= GLFW.GLFW_KEY_A && key <= GLFW.GLFW_KEY_Z) return String.valueOf((char) (key - GLFW.GLFW_KEY_A + 1));
        return switch (key) {
            case GLFW.GLFW_KEY_UP -> "\u001b[A";
            case GLFW.GLFW_KEY_DOWN -> "\u001b[B";
            case GLFW.GLFW_KEY_RIGHT -> "\u001b[C";
            case GLFW.GLFW_KEY_LEFT -> "\u001b[D";
            case GLFW.GLFW_KEY_HOME -> "\u001b[H";
            case GLFW.GLFW_KEY_END -> "\u001b[F";
            case GLFW.GLFW_KEY_PAGE_UP -> "\u001b[5~";
            case GLFW.GLFW_KEY_PAGE_DOWN -> "\u001b[6~";
            case GLFW.GLFW_KEY_DELETE -> "\u001b[3~";
            case GLFW.GLFW_KEY_ENTER, GLFW.GLFW_KEY_KP_ENTER -> "\n";
            case GLFW.GLFW_KEY_BACKSPACE -> "\b";
            case GLFW.GLFW_KEY_TAB -> "\t";
            default -> null;
        };
    }

    @Override
    public boolean charTyped(char c, int mods) {
        if (c >= 32 && c != 127) {
            type(String.valueOf(c));
            if (phone != null) PacketDistributor.sendToServer(com.example.evanscomputermod.network.KeyInputPacket.text(phone,
                    (byte) IComputerHost.DISPLAY_OWN, String.valueOf(c)));
        }
        return true;
    }

    private void type(String s) {
        if (phone == null) return;
        keysSent++;
        PacketDistributor.sendToServer(new TerminalInputPacket(phone, (byte) IComputerHost.DISPLAY_OWN, s));
    }

    /** Where phone pixel ({@code px}, {@code py}) of the panel's portrait mode is on this screen (render checks). */
    public double[] screenPoint(int px, int py) {
        PanelTier tier = tier();
        int[] size = PhoneScreenTexture.size(tier != null ? tier : PanelTier.COLOR_LCD);
        return new double[]{sx + (px + 0.5) * sw / size[0], sy + (py + 0.5) * sh / size[1]};
    }

    /** Input packets this screen sent (render checks). */
    public int keysSent() {
        return keysSent;
    }

    // ------------------------------------------------------------ mouse as touch

    /** The phone pixel under the mouse (graphics pixels when graphics show), or null outside the screen. */
    @Nullable
    private int[] pixel(double mx, double my) {
        // While a button is held (a drag) a point off the screen maps to its nearest edge.
        if (phone != null && buttons != 0) {
            mx = Math.max(sx, Math.min(sx + sw - 0.001, mx));
            my = Math.max(sy, Math.min(sy + sh - 0.001, my));
        }
        if (phone == null || mx < sx || my < sy || mx >= sx + sw || my >= sy + sh) return null;
        TerminalDisplay d = ClientDisplayCache.peek(phone, IComputerHost.DISPLAY_OWN);
        PanelTier tier = tier();
        int[] size = PhoneScreenTexture.size(tier != null ? tier : PanelTier.COLOR_LCD);
        int w = size[0], h = size[1];
        double fx = (mx - sx) / sw, fy = (my - sy) / sh;
        if (d != null && d.getDisplayMode() >= 1 && d.getGfxWidth() > 0 && d.getGfxHeight() > 0) {
            // The picture is letterboxed into the screen the way PhoneScreenTexture draws it.
            int gw = d.getGfxWidth(), gh = d.getGfxHeight();
            double s = Math.min(w / (double) gw, h / (double) gh);
            double ox = (w - gw * s) / 2, oy = (h - gh * s) / 2;
            double px = (fx * w - ox) / s, py = (fy * h - oy) / s;
            if (buttons != 0) {
                px = Math.max(0, Math.min(gw - 1, px));
                py = Math.max(0, Math.min(gh - 1, py));
            }
            if (px < 0 || py < 0 || px >= gw || py >= gh) return null;
            return new int[]{(int) px, (int) py};
        }
        return new int[]{Math.min(w - 1, (int) (fx * w)), Math.min(h - 1, (int) (fy * h))};
    }

    private void pointer(byte kind, double mx, double my, int button, byte scroll) {
        int[] p = pixel(mx, my);
        if (p == null || phone == null) return;
        PacketDistributor.sendToServer(new MouseInputPacket(phone, (byte) IComputerHost.DISPLAY_OWN, kind, (short) p[0], (short) p[1],
                buttons, (byte) button, scroll));
    }

    private static byte bit(int button) {
        return switch (button) {
            case 0 -> 1;
            case 1 -> 2;
            case 2 -> 4;
            default -> 0;
        };
    }

    @Override
    public boolean mouseClicked(double mx, double my, int button) {
        buttons |= bit(button);
        pointer(MouseInputPacket.KIND_DOWN, mx, my, button, (byte) 0);
        return true;
    }

    @Override
    public boolean mouseReleased(double mx, double my, int button) {
        // Map the point while the button still counts as held, so a release
        // off the screen still reaches the phone (and ends the drag there).
        int[] p = pixel(mx, my);
        buttons &= (byte) ~bit(button);
        if (p != null && phone != null) {
            PacketDistributor.sendToServer(new MouseInputPacket(phone, (byte) IComputerHost.DISPLAY_OWN, MouseInputPacket.KIND_UP,
                    (short) p[0], (short) p[1], buttons, (byte) button, (byte) 0));
        }
        return true;
    }

    @Override
    public void mouseMoved(double mx, double my) {
        int[] p = pixel(mx, my);
        if (p == null || (p[0] == lastX && p[1] == lastY)) return;
        lastX = p[0];
        lastY = p[1];
        pointer(MouseInputPacket.KIND_MOVE, mx, my, 0, (byte) 0);
    }

    @Override
    public boolean mouseScrolled(double mx, double my, double sxAmount, double syAmount) {
        if (syAmount != 0) pointer(MouseInputPacket.KIND_SCROLL, mx, my, 0, (byte) (syAmount > 0 ? 1 : -1));
        return true;
    }

    @Override
    public void resize(Minecraft mc, int w, int h) {
        super.resize(mc, w, h);
        layout();
    }
}
//?}
