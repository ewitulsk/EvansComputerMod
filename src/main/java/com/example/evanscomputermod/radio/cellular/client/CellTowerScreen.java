package com.example.evanscomputermod.radio.cellular.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.cellular.CarrierRegistry;
import com.example.evanscomputermod.radio.cellular.CellPackets;
import com.example.evanscomputermod.radio.cellular.CellTech;
import com.example.evanscomputermod.radio.cellular.CellTowerMenu;
import com.example.evanscomputermod.radio.cellular.SimCard;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.entity.player.Inventory;
import net.neoforged.neoforge.network.PacketDistributor;
import org.lwjgl.glfw.GLFW;

import java.util.ArrayList;
import java.util.List;
import java.util.Locale;

/**
 * The carrier screen of a Cell Tower: the tower's carrier and channels, what
 * each technology's antennas give, and the carrier's SIMs (which are attached
 * here, their signal and data use) with a per-SIM data limit. Issue SIM turns
 * a blank SIM card from your inventory into one of this carrier's.
 */
public class CellTowerScreen extends AbstractContainerScreen<CellTowerMenu> {
    private static final int W = 300, H = 232, ROWS = 7;
    private static final int BG = 0xF0101418, PANEL = 0xFF1C232B, BORDER = 0xFF3D8BD9, TEXT = 0xFFE6EDF3, DIM = 0xFF8B98A5,
            SEL = 0xFF27405A, GOOD = 0xFF7EE787, BAD = 0xFFFF7B72;

    private EditBox carrier, label, quota;
    private CycleButton<Integer> lte, mm;
    private long selected = -1;

    public CellTowerScreen(CellTowerMenu menu, Inventory inv, Component title) {
        super(menu, inv, title);
        imageWidth = W;
        imageHeight = H;
    }

    @Override
    protected void init() {
        super.init();
        var v = menu.view();
        int x = leftPos + 8, y = topPos + 20;
        carrier = new EditBox(font, x + 46, y, 100, 16, Component.translatable("gui.evanscomputermod.cell_tower.carrier"));
        carrier.setMaxLength(24);
        carrier.setValue(v.carrier());
        lte = channelButton(CellTech.LTE, v.lteChannel(), x + 150, y);
        mm = channelButton(CellTech.MMWAVE, v.mmChannel(), x + 150, y + 18);
        addRenderableWidget(carrier);
        addRenderableWidget(lte);
        addRenderableWidget(mm);
        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.cell_tower.apply"),
                b -> send(CellPackets.Action.apply(v.pos(), carrier.getValue(), lte.getValue(), mm.getValue())))
                .bounds(x + 46, y + 18, 100, 16).build());

        int by = topPos + H - 40;
        label = new EditBox(font, x, by, 100, 16, Component.translatable("gui.evanscomputermod.cell_tower.sim_label"));
        label.setMaxLength(32);
        label.setHint(Component.literal("SIM label").withColor(DIM));
        addRenderableWidget(label);
        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.cell_tower.issue"),
                b -> send(CellPackets.Action.issue(menu.view().pos(), label.getValue()))).bounds(x + 104, by, 64, 16).build());
        quota = new EditBox(font, x + 176, by, 50, 16, Component.translatable("gui.evanscomputermod.cell_tower.quota"));
        quota.setMaxLength(9);
        quota.setHint(Component.literal("MB").withColor(DIM));
        addRenderableWidget(quota);
        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.cell_tower.set_quota"), b -> setQuota())
                .bounds(x + 230, by, 54, 16).build());
    }

    private CycleButton<Integer> channelButton(CellTech t, int initial, int x, int y) {
        List<Integer> chans = new ArrayList<>();
        for (int c = 1; c <= t.channels; c++) chans.add(c);
        return CycleButton.<Integer>builder(c -> Component.literal(String.format(Locale.ROOT, "%d (%.0f MHz)", c, t.downlink(c).centerHz() / 1e6)))
                .withValues(chans).withInitialValue(t.validChannel(initial) ? initial : 1)
                .create(x, y, 134, 16, Component.literal(t.displayName));
    }

    private void setQuota() {
        if (selected < 0) {
            menu.setView(withMessage("Click a SIM in the list first"));
            return;
        }
        String q = quota.getValue().trim();
        long bytes;
        if (q.isEmpty() || q.equalsIgnoreCase("off") || q.equals("-")) bytes = CarrierRegistry.NO_QUOTA;
        else {
            try {
                bytes = Math.round(Double.parseDouble(q) * 1_000_000);
            } catch (NumberFormatException e) {
                menu.setView(withMessage("Data limit: a number of MB, or empty for none"));
                return;
            }
        }
        send(CellPackets.Action.quota(menu.view().pos(), selected, bytes));
    }

    private void send(CellPackets.Action a) {
        PacketDistributor.sendToServer(a);
    }

    private CellPackets.TowerView withMessage(String msg) {
        var v = menu.view();
        return new CellPackets.TowerView(v.pos(), v.carrier(), v.owner(), v.carrierOwner(), v.lteChannel(), v.mmChannel(), v.cabled(),
                v.techs(), v.sims(), msg);
    }

    static String bytes(long b) {
        if (b < 0) return "no limit";
        if (b < 10_000) return b + " B";
        if (b < 10_000_000) return String.format(Locale.ROOT, "%.1f kB", b / 1e3);
        return String.format(Locale.ROOT, "%.1f MB", b / 1e6);
    }

    @Override
    public boolean mouseClicked(double mx, double my, int button) {
        var v = menu.view();
        int rowTop = topPos + 96;
        for (int i = 0; i < Math.min(ROWS, v.sims().size()); i++) {
            int y = rowTop + i * 12;
            if (mx >= leftPos + 6 && mx < leftPos + W - 6 && my >= y - 1 && my < y + 11) {
                selected = v.sims().get(i).imsi();
                long q = v.sims().get(i).quotaBytes();
                quota.setValue(q < 0 ? "" : String.format(Locale.ROOT, "%.3f", q / 1e6).replaceAll("\\.?0+$", ""));
                return true;
            }
        }
        return super.mouseClicked(mx, my, button);
    }

    @Override
    protected void renderBg(GuiGraphics g, float partialTick, int mouseX, int mouseY) {
        g.fill(leftPos - 1, topPos - 1, leftPos + W + 1, topPos + H + 1, BORDER);
        g.fill(leftPos, topPos, leftPos + W, topPos + H, BG);
        g.fill(leftPos + 4, topPos + 82, leftPos + W - 4, topPos + H - 44, PANEL);
        var v = menu.view();
        for (int i = 0; i < Math.min(ROWS, v.sims().size()); i++)
            if (v.sims().get(i).imsi() == selected) g.fill(leftPos + 5, topPos + 95 + i * 12, leftPos + W - 5, topPos + 107 + i * 12, SEL);
    }

    @Override
    protected void renderLabels(GuiGraphics g, int mouseX, int mouseY) {
        var v = menu.view();
        g.drawString(font, title, 8, 7, TEXT, false);
        String who = "owner " + (v.owner().isEmpty() ? "-" : v.owner()) + (v.cabled() ? ", cabled" : ", NOT cabled");
        g.drawString(font, who, W - 8 - font.width(who), 7, v.cabled() ? DIM : 0xFFFFB454, false);
        g.drawString(font, Component.translatable("gui.evanscomputermod.cell_tower.carrier"), 8, 24, DIM, false);
        int y = 58;
        for (CellPackets.TechRow t : v.techs()) {
            String line = t.antennas() == 0 ? t.name() + ": no antennas on the mast"
                    : String.format(Locale.ROOT, "%s: ch %d %.0f MHz, %d antenna%s at %d m, %s, %d attached", t.name(), t.channel(),
                    t.freqMhz(), t.antennas(), t.antennas() == 1 ? "" : "s", t.heightM(), t.onAir() ? "on air" : "off air", t.devices());
            g.drawString(font, font.plainSubstrByWidth(line, W - 16), 8, y, t.onAir() ? TEXT : DIM, false);
            y += 11;
        }
        String head = v.carrier().isEmpty() ? "No carrier" : "SIMs of " + v.carrier() + (v.carrierOwner().isEmpty() ? "" : " (" + v.carrierOwner() + ")");
        g.drawString(font, head + "  -  IMSI / attached / signal / used of limit", 8, 85, DIM, false);
        if (v.sims().isEmpty()) g.drawString(font, "none issued yet: put a blank SIM card in your inventory and press Issue SIM", 8, 97, DIM, false);
        for (int i = 0; i < Math.min(ROWS, v.sims().size()); i++) {
            CellPackets.SimRow s = v.sims().get(i);
            String on = s.attached().isEmpty() ? "-" : s.attached() + (Double.isNaN(s.rssiDbm()) ? "" : String.format(Locale.ROOT, " %.0f dBm CQI %d", s.rssiDbm(), s.cqi()));
            String line = String.format(Locale.ROOT, "%-10s %s  %s  %s / %s%s", trim(s.label(), 10), SimCard.formatImsi(s.imsi()), on,
                    bytes(s.usedBytes()), bytes(s.quotaBytes()), s.revoked() ? " REVOKED" : "");
            boolean over = s.quotaBytes() >= 0 && s.usedBytes() >= s.quotaBytes();
            g.drawString(font, font.plainSubstrByWidth(line, W - 16), 8, 97 + i * 12, over ? BAD : s.attached().isEmpty() ? DIM : TEXT, false);
        }
        if (!v.message().isEmpty())
            g.drawString(font, font.plainSubstrByWidth(v.message(), W - 16), 8, H - 18,
                    v.message().startsWith("Not") || v.message().startsWith("Only") || v.message().startsWith("You") ? BAD : GOOD, false);
    }

    private static String trim(String s, int n) {
        return s.length() <= n ? s : s.substring(0, n - 1) + "~";
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partialTick) {
        super.render(g, mouseX, mouseY, partialTick);
        renderTooltip(g, mouseX, mouseY);
    }

    @Override
    public boolean keyPressed(int key, int scan, int mods) {
        GuiEventListener f = getFocused();
        if (key != GLFW.GLFW_KEY_ESCAPE && f instanceof EditBox box && box.isFocused()) {
            return box.keyPressed(key, scan, mods) || box.canConsumeInput();
        }
        return super.keyPressed(key, scan, mods);
    }
}
//?}
