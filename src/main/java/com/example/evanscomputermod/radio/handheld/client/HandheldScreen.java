package com.example.evanscomputermod.radio.handheld.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.handheld.HandheldBand;
import com.example.evanscomputermod.radio.handheld.HandheldPackets;
import com.example.evanscomputermod.radio.handheld.HandheldRadioItem;
import com.example.evanscomputermod.radio.handheld.HandheldSettings;
import com.example.evanscomputermod.speaker.client.HandheldAudioClient;
import net.minecraft.client.Minecraft;
import net.minecraft.client.gui.GuiGraphics;
import net.minecraft.client.gui.components.Button;
import net.minecraft.client.gui.components.EditBox;
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.neoforged.neoforge.network.PacketDistributor;
import org.lwjgl.glfw.GLFW;

import java.util.Locale;

/**
 * Handheld tuning screen: a frequency box (type it, press Enter), frequency
 * dial (fine/coarse), band switch, seek ("Scan", done by the server, which
 * jumps straight to the next station it can hear), volume, squelch, power,
 * and a live signal meter. Changes go to the server, which stores them on the item.
 */
public class HandheldScreen extends Screen {

    private final InteractionHand hand;
    private HandheldSettings s;
    private EditBox freqBox;
    private String status = "";
    private int statusColor = 0xAAAAAA;

    public static void open(InteractionHand hand) {
        Minecraft.getInstance().setScreen(new HandheldScreen(hand));
    }

    HandheldScreen(InteractionHand hand) {
        super(Component.translatable("item.evanscomputermod.handheld_radio"));
        this.hand = hand;
        this.s = HandheldSettings.read(Minecraft.getInstance().player.getItemInHand(hand));
    }

    /** The server's answer to Scan. */
    public static void onSeekResult(HandheldPackets.SeekResult r) {
        if (!(Minecraft.getInstance().screen instanceof HandheldScreen screen)) return;
        if (r.found()) {
            screen.s = screen.s.withFreq(r.freqHz());
            screen.setStatus("Station at " + HandheldRadioItem.freqLabel(r.freqHz()), 0x7CFC00);
        } else {
            screen.setStatus("No stations on " + screen.s.band() + " here", 0xFF8080);
        }
    }

    @Override
    protected void init() {
        int cx = width / 2, y = height / 2 - 40;
        freqBox = new EditBox(font, cx - 70, y - 66, 84, 18, Component.literal("Frequency"));
        freqBox.setHint(Component.literal("e.g. 11.6"));
        freqBox.setMaxLength(16);
        addRenderableWidget(freqBox);
        addRenderableWidget(Button.builder(Component.literal("Tune"), b -> tuneTyped()).bounds(cx + 18, y - 67, 52, 20).build());
        setInitialFocus(freqBox);
        addRenderableWidget(Button.builder(Component.literal("<<"), b -> step(-10)).bounds(cx - 60, y, 30, 20).build());
        addRenderableWidget(Button.builder(Component.literal("<"), b -> step(-1)).bounds(cx - 26, y, 24, 20).build());
        addRenderableWidget(Button.builder(Component.literal(">"), b -> step(1)).bounds(cx + 2, y, 24, 20).build());
        addRenderableWidget(Button.builder(Component.literal(">>"), b -> step(10)).bounds(cx + 30, y, 30, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Band"), b -> send(s.withBand(s.band().next()))).bounds(cx - 110, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Scan -"), b -> seek(-1)).bounds(cx - 56, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Scan +"), b -> seek(1)).bounds(cx - 2, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Power"), b -> send(s.withOn(!s.on()))).bounds(cx + 52, y + 28, 58, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Vol -"), b -> send(s.withVolume(s.volume() - 10))).bounds(cx - 110, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Vol +"), b -> send(s.withVolume(s.volume() + 10))).bounds(cx - 56, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Sql -"), b -> send(s.withSquelch(s.squelch() - 10))).bounds(cx - 2, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Sql +"), b -> send(s.withSquelch(s.squelch() + 10))).bounds(cx + 52, y + 56, 58, 20).build());
    }

    @Override
    public boolean keyPressed(int key, int scan, int mods) {
        if ((key == GLFW.GLFW_KEY_ENTER || key == GLFW.GLFW_KEY_KP_ENTER) && freqBox.isFocused()) {
            tuneTyped();
            return true;
        }
        return super.keyPressed(key, scan, mods);
    }

    /**
     * Parse a typed frequency: a number with an optional unit (k, kHz, M, MHz, Hz).
     * Without a unit, values from 1000 up are kHz (AM dial: "1000"), smaller ones MHz ("11.6",
     * "146.52"). Returns Hz, or NaN.
     */
    public static double parseFrequency(String text) {
        String t = text.trim().toLowerCase(Locale.ROOT).replace(" ", "").replace(',', '.');
        double mult = Double.NaN;
        for (String[] u : new String[][] {{"mhz", "1e6"}, {"khz", "1e3"}, {"hz", "1"}, {"m", "1e6"}, {"k", "1e3"}}) {
            if (t.endsWith(u[0])) {
                t = t.substring(0, t.length() - u[0].length());
                mult = Double.parseDouble(u[1]);
                break;
            }
        }
        double v;
        try {
            v = Double.parseDouble(t);
        } catch (NumberFormatException e) {
            return Double.NaN;
        }
        if (!(v > 0)) return Double.NaN;
        if (Double.isNaN(mult)) mult = v >= 1000 ? 1e3 : 1e6;
        return v * mult;
    }

    /** The band that covers {@code hz}, or null. */
    public static HandheldBand bandFor(double hz) {
        for (HandheldBand b : HandheldBand.values()) if (hz >= b.minHz && hz <= b.maxHz) return b;
        return null;
    }

    private void tuneTyped() {
        double hz = parseFrequency(freqBox.getValue());
        if (Double.isNaN(hz)) {
            setStatus("Type a frequency like 11.6 (MHz) or 1000k", 0xFF8080);
            return;
        }
        HandheldBand band = bandFor(hz);
        if (band == null) {
            setStatus("Out of range: AM 530-1700 kHz, SW 3-30 MHz, VHF 30-300 MHz", 0xFF8080);
            return;
        }
        // Nearest 1 kHz, not the step grid: steps count from 0 Hz, so 146.52 MHz isn't on the 12.5 kHz grid.
        double snapped = com.example.evanscomputermod.radio.handheld.HandheldServer.snap(band, hz);
        send(new HandheldSettings(s.on(), band, snapped, s.volume(), s.squelch()));
        freqBox.setValue("");
        setStatus("Tuned to " + band + " " + HandheldRadioItem.freqLabel(snapped) + (s.on() ? "" : " (radio is off: press Power)"),
                s.on() ? 0x7CFC00 : 0xFFD060);
    }

    private void setStatus(String text, int color) {
        status = text;
        statusColor = color;
    }

    private void step(int steps) {
        send(s.withFreq(s.freqHz() + steps * s.band().step(s.freqHz())));
    }

    private void seek(int direction) {
        setStatus("Scanning " + s.band() + "...", 0xAAAAAA);
        PacketDistributor.sendToServer(new HandheldPackets.Seek(hand == InteractionHand.MAIN_HAND, direction));
    }

    private void send(HandheldSettings next) {
        s = next;
        PacketDistributor.sendToServer(new HandheldPackets.Settings(hand == InteractionHand.MAIN_HAND, s.on(),
                s.band().ordinal(), s.freqHz(), s.volume(), s.squelch()));
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partial) {
        super.render(g, mouseX, mouseY, partial);
        int cx = width / 2, y = height / 2 - 40;
        g.drawCenteredString(font, s.band() + "  " + HandheldRadioItem.freqLabel(s.freqHz()) + (s.on() ? "" : "  (off)"), cx, y - 16, 0x7CFC00);
        float dbm = HandheldAudioClient.signalDbm;
        int bars = dbm < -125 ? 0 : (int) Math.min(10, (dbm + 125) / 6);
        g.drawCenteredString(font, "S " + "|".repeat(bars) + ".".repeat(10 - bars) + (dbm > -199 ? String.format("  %.0f dBm", dbm) : ""), cx, y - 38, 0xFFFFFF);
        g.drawCenteredString(font, "Volume " + s.volume() + "   Squelch " + s.squelch(), cx, y + 84, 0xAAAAAA);
        if (!status.isEmpty()) g.drawCenteredString(font, status, cx, y + 98, statusColor);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }
}
//?}
