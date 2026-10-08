package com.example.evanscomputermod.radio.wifi.ap.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.wifi.ap.AccessPointMenu;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import com.example.evanscomputermod.radio.wifi.ap.ApSettings;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.CycleButton;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.components.events.GuiEventListener;
import net.minecraft.client.gui.screens.inventory.AbstractContainerScreen;
import net.minecraft.network.chat.Component;
import net.minecraft.network.chat.Style;
import net.minecraft.util.FormattedCharSequence;
import net.minecraft.world.entity.player.Inventory;
import net.neoforged.neoforge.network.PacketDistributor;
import org.lwjgl.glfw.GLFW;

import java.util.ArrayList;
import java.util.Arrays;
import java.util.List;

/**
 * Access Point settings and status. Settings: SSID, hidden, security,
 * passphrase (write-only: the field starts empty and leaving it empty keeps
 * the stored one), channel, transmit power, client isolation and the MAC
 * filter. Status: BSSID, channel in use, cable link, and every associated
 * client with RSSI, rate, handshake state and last error (refreshed live).
 */
public class AccessPointScreen extends AbstractContainerScreen<AccessPointMenu> {
    private static final int W = 280, H = 214;
    private static final int BG = 0xF0101418, PANEL = 0xFF1C232B, BORDER = 0xFF3D8BD9, TEXT = 0xFFE6EDF3, DIM = 0xFF8B98A5;
    private static final List<Integer> TX_STEPS = List.of(0, 3, 6, 10, 13, 15, 17, 20);

    private boolean statusPage;
    private EditBox ssid, passphrase, macs;
    private CycleButton<Boolean> hidden, isolation;
    private CycleButton<Security> security;
    private CycleButton<Integer> channel, txPower;
    private CycleButton<ApConfig.MacFilterMode> filterMode;
    private final List<Button> kickButtons = new ArrayList<>();

    public AccessPointScreen(AccessPointMenu menu, Inventory inv, Component title) {
        super(menu, inv, title);
        imageWidth = W;
        imageHeight = H;
    }

    @Override
    protected void init() {
        super.init();
        ApSettings s = menu.view().settings();
        int x = leftPos + 96, y = topPos + 24, w = 172;
        ssid = new EditBox(font, x, y, w, 16, Component.translatable("gui.evanscomputermod.access_point.ssid"));
        ssid.setMaxLength(32);
        ssid.setValue(s.ssid());
        hidden = CycleButton.onOffBuilder(s.hidden()).create(x, y + 20, w, 16, Component.translatable("gui.evanscomputermod.access_point.hidden"));
        security = CycleButton.<Security>builder(v -> Component.literal(v == Security.OPEN ? "Open" : "WPA2-PSK (AES-CCMP)"))
                .withValues(Security.values()).withInitialValue(s.security())
                .create(x, y + 40, w, 16, Component.translatable("gui.evanscomputermod.access_point.security"));
        passphrase = new EditBox(font, x, y + 60, w, 16, Component.translatable("gui.evanscomputermod.access_point.passphrase"));
        passphrase.setMaxLength(64);
        passphrase.setFormatter((text, start) -> FormattedCharSequence.forward("*".repeat(text.length()), Style.EMPTY));
        passphrase.setHint(Component.literal(menu.view().hasPassphrase() ? "(unchanged)" : "(not set)").withColor(DIM));
        List<Integer> chans = Arrays.stream(ApSettings.CHANNEL_CHOICES).boxed().toList();
        channel = CycleButton.<Integer>builder(c -> Component.literal(c == 0 ? "Auto (1/6/11)" : c <= 14 ? c + " (2.4 GHz)" : c + " (5 GHz)"))
                .withValues(chans).withInitialValue(chans.contains(s.channelSetting()) ? s.channelSetting() : 0)
                .create(x, y + 80, w, 16, Component.translatable("gui.evanscomputermod.access_point.channel"));
        List<Integer> tx = new ArrayList<>(TX_STEPS);
        if (!tx.contains(s.txPowerDbm())) tx.add(s.txPowerDbm());
        tx.sort(Integer::compare);
        txPower = CycleButton.<Integer>builder(p -> Component.literal(p + " dBm (" + mw(p) + ")"))
                .withValues(tx).withInitialValue(s.txPowerDbm())
                .create(x, y + 100, w, 16, Component.translatable("gui.evanscomputermod.access_point.tx_power"));
        isolation = CycleButton.onOffBuilder(s.clientIsolation()).create(x, y + 120, w, 16,
                Component.translatable("gui.evanscomputermod.access_point.isolation"));
        filterMode = CycleButton.<ApConfig.MacFilterMode>builder(m -> Component.literal(switch (m) {
                    case OFF -> "Off";
                    case ALLOW -> "Allow only listed";
                    case DENY -> "Deny listed";
                })).withValues(ApConfig.MacFilterMode.values()).withInitialValue(s.filterMode())
                .create(x, y + 140, w, 16, Component.translatable("gui.evanscomputermod.access_point.mac_filter"));
        macs = new EditBox(font, x, y + 160, w, 16, Component.translatable("gui.evanscomputermod.access_point.mac_list"));
        macs.setMaxLength(ApSettings.MAX_FILTER_MACS * 19);
        macs.setValue(ApSettings.formatMacList(s.filterMacs()));
        macs.setHint(Component.literal("aa:bb:cc:dd:ee:ff, ...").withColor(DIM));
        for (var wdg : List.of(ssid, hidden, security, passphrase, channel, txPower, isolation, filterMode, macs)) addRenderableWidget(wdg);

        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.access_point.apply"), b -> apply())
                .bounds(leftPos + W - 70, topPos + H - 22, 60, 16).build());
        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.access_point.settings_tab"), b -> setPage(false))
                .bounds(leftPos + 8, topPos + 4, 70, 14).build());
        addRenderableWidget(Button.builder(Component.translatable("gui.evanscomputermod.access_point.status_tab"), b -> setPage(true))
                .bounds(leftPos + 82, topPos + 4, 70, 14).build());
        setPage(statusPage);
    }

    private static String mw(int dbm) {
        double mw = Math.pow(10, dbm / 10.0);
        return mw >= 10 ? Math.round(mw) + " mW" : String.format("%.1f mW", mw);
    }

    private void setPage(boolean status) {
        statusPage = status;
        for (var wdg : List.of(ssid, hidden, security, passphrase, channel, txPower, isolation, filterMode, macs)) wdg.visible = !status;
        rebuildKickButtons();
    }

    private void rebuildKickButtons() {
        for (Button b : kickButtons) removeWidget(b);
        kickButtons.clear();
        if (!statusPage) return;
        List<ApPackets.ClientRow> rows = menu.view().clients();
        for (int i = 0; i < Math.min(rows.size(), 6); i++) {
            String mac = rows.get(i).mac();
            Button b = Button.builder(Component.literal("x"), btn -> send(ApPackets.Action.KICK, MacAddress.parse(mac).value()))
                    .bounds(leftPos + W - 20, topPos + 71 + i * 20, 12, 12).build();
            b.setTooltip(net.minecraft.client.gui.components.Tooltip.create(Component.translatable("gui.evanscomputermod.access_point.kick")));
            kickButtons.add(addRenderableWidget(b));
        }
    }

    private void apply() {
        send(ApPackets.Action.APPLY, 0);
        passphrase.setValue("");
    }

    private void send(ApPackets.Action action, long kickMac) {
        List<MacAddress> list;
        try {
            list = ApSettings.parseMacList(macs.getValue());
        } catch (IllegalArgumentException e) {
            menu.setView(withMessage(e.getMessage()));
            return;
        }
        if (list.size() > ApSettings.MAX_FILTER_MACS) list = list.subList(0, ApSettings.MAX_FILTER_MACS);
        ApSettings s = new ApSettings(ssid.getValue(), hidden.getValue(), security.getValue(), channel.getValue(), txPower.getValue(),
                isolation.getValue(), filterMode.getValue(), list);
        PacketDistributor.sendToServer(new ApPackets.Configure(menu.view().pos(), action, s, passphrase.getValue(), kickMac));
    }

    private ApPackets.ApView withMessage(String msg) {
        var v = menu.view();
        return new ApPackets.ApView(v.pos(), v.settings(), v.hasPassphrase(), v.bssid(), v.portMac(), v.channelInUse(), v.cabled(),
                v.radioOn(), v.owner(), v.clients(), v.events(), msg, v.radioFrames(), v.wiredIn(), v.wiredOut());
    }

    private int shownClients = -1;

    @Override
    public void containerTick() {
        super.containerTick();
        if (statusPage && shownClients != menu.view().clients().size()) {
            shownClients = menu.view().clients().size();
            rebuildKickButtons();
        }
    }

    @Override
    protected void renderBg(GuiGraphics g, float partialTick, int mouseX, int mouseY) {
        g.fill(leftPos - 1, topPos - 1, leftPos + W + 1, topPos + H + 1, BORDER);
        g.fill(leftPos, topPos, leftPos + W, topPos + H, BG);
        g.fill(leftPos + 4, topPos + 20, leftPos + W - 4, topPos + H - 26, PANEL);
    }

    @Override
    protected void renderLabels(GuiGraphics g, int mouseX, int mouseY) {
        g.drawString(font, title, W - 8 - font.width(title), 7, TEXT, false);
        var v = menu.view();
        if (!statusPage) {
            String[] labels = {"ssid", "hidden", "security", "passphrase", "channel", "tx_power", "isolation", "mac_filter", "mac_list"};
            for (int i = 0; i < labels.length; i++)
                g.drawString(font, Component.translatable("gui.evanscomputermod.access_point." + labels[i]), 10, 28 + i * 20, DIM, false);
        } else {
            int y = 26;
            g.drawString(font, "BSSID " + v.bssid() + "   port " + v.portMac(), 10, y, TEXT, false);
            y += 11;
            String radio = v.radioOn() ? "on, channel " + v.channelInUse() + (v.settings().channelSetting() == 0 ? " (auto)" : "") : "off";
            g.drawString(font, "Radio " + radio + "   cable " + (v.cabled() ? "connected" : "NOT connected")
                    + "   SSID " + v.settings().ssid(), 10, y, v.cabled() ? TEXT : 0xFFFFB454, false);
            y += 11;
            g.drawString(font, "Frames: " + v.radioFrames() + " on air, " + v.wiredIn() + " from cable, " + v.wiredOut() + " to cable",
                    10, y, DIM, false);
            y += 14;
            g.drawString(font, "Clients (" + v.clients().size() + ")      RSSI   rate   state / handshake", 10, y, DIM, false);
            y += 12;
            if (v.clients().isEmpty()) g.drawString(font, "none associated", 14, y, DIM, false);
            for (int i = 0; i < Math.min(v.clients().size(), 6); i++) {
                var c = v.clients().get(i);
                String rate = c.rateKbps() == 0 ? "-" : (c.rateKbps() % 1000 == 0 ? c.rateKbps() / 1000 + "" : String.format("%.1f", c.rateKbps() / 1000.0)) + "M";
                g.drawString(font, c.mac() + "  " + c.rssiDbm() + " dBm  " + rate + "  " + c.state().toLowerCase() + " / "
                        + c.handshake().toLowerCase(), 10, y + i * 20, TEXT, false);
                if (!c.lastError().isEmpty()) g.drawString(font, "  " + c.lastError(), 10, y + i * 20 + 9, 0xFFFF7B72, false);
            }
            int ey = H - 26 - 10 * Math.min(3, v.events().size()) - 2;
            List<String> ev = v.events();
            for (int i = Math.max(0, ev.size() - 3); i < ev.size(); i++, ey += 10)
                g.drawString(font, font.plainSubstrByWidth(ev.get(i), W - 20), 10, ey, DIM, false);
        }
        if (!v.message().isEmpty())
            g.drawString(font, font.plainSubstrByWidth(v.message(), W - 90), 8, H - 18,
                    v.message().startsWith("Not") || v.message().startsWith("Only") ? 0xFFFF7B72 : 0xFF7EE787, false);
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
