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
import net.minecraft.client.gui.screens.Screen;
import net.minecraft.network.chat.Component;
import net.minecraft.world.InteractionHand;
import net.neoforged.neoforge.network.PacketDistributor;

/**
 * Handheld tuning screen: frequency dial (fine/coarse), band switch, scan,
 * volume, squelch, power, and a live signal meter. Changes go to the server,
 * which stores them on the item.
 */
public class HandheldScreen extends Screen {

    private final InteractionHand hand;
    private HandheldSettings s;
    private int scanDir;

    public static void open(InteractionHand hand) {
        Minecraft.getInstance().setScreen(new HandheldScreen(hand));
    }

    HandheldScreen(InteractionHand hand) {
        super(Component.translatable("item.evanscomputermod.handheld_radio"));
        this.hand = hand;
        this.s = HandheldSettings.read(Minecraft.getInstance().player.getItemInHand(hand));
    }

    @Override
    protected void init() {
        int cx = width / 2, y = height / 2 - 40;
        addRenderableWidget(Button.builder(Component.literal("<<"), b -> step(-10)).bounds(cx - 110, y, 30, 20).build());
        addRenderableWidget(Button.builder(Component.literal("<"), b -> step(-1)).bounds(cx - 76, y, 24, 20).build());
        addRenderableWidget(Button.builder(Component.literal(">"), b -> step(1)).bounds(cx + 52, y, 24, 20).build());
        addRenderableWidget(Button.builder(Component.literal(">>"), b -> step(10)).bounds(cx + 80, y, 30, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Band"), b -> send(s.withBand(s.band().next()))).bounds(cx - 110, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Scan -"), b -> scanDir = -1).bounds(cx - 56, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Scan +"), b -> scanDir = 1).bounds(cx - 2, y + 28, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Power"), b -> send(s.withOn(!s.on()))).bounds(cx + 52, y + 28, 58, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Vol -"), b -> send(s.withVolume(s.volume() - 10))).bounds(cx - 110, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Vol +"), b -> send(s.withVolume(s.volume() + 10))).bounds(cx - 56, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Sql -"), b -> send(s.withSquelch(s.squelch() - 10))).bounds(cx - 2, y + 56, 50, 20).build());
        addRenderableWidget(Button.builder(Component.literal("Sql +"), b -> send(s.withSquelch(s.squelch() + 10))).bounds(cx + 52, y + 56, 58, 20).build());
    }

    private void step(int steps) {
        scanDir = 0;
        send(s.withFreq(s.freqHz() + steps * s.band().step(s.freqHz())));
    }

    private void send(HandheldSettings next) {
        s = next;
        PacketDistributor.sendToServer(new HandheldPackets.Settings(hand == InteractionHand.MAIN_HAND, s.on(),
                s.band().ordinal(), s.freqHz(), s.volume(), s.squelch()));
    }

    @Override
    public void tick() {
        // Scan: step every few ticks until a signal clearly above the noise appears.
        if (scanDir != 0 && minecraft.level.getGameTime() % 4 == 0) {
            if (HandheldAudioClient.signalDbm > -100 && HandheldAudioClient.freqHz == s.freqHz()) scanDir = 0;
            else {
                double next = s.freqHz() + scanDir * s.band().step(s.freqHz());
                if (next > s.band().maxHz || next < s.band().minHz) scanDir = 0;
                else send(s.withFreq(next));
            }
        }
    }

    @Override
    public void render(GuiGraphics g, int mouseX, int mouseY, float partial) {
        super.render(g, mouseX, mouseY, partial);
        int cx = width / 2, y = height / 2 - 40;
        g.drawCenteredString(font, s.band() + "  " + HandheldRadioItem.freqLabel(s.freqHz()) + (s.on() ? "" : "  (off)"), cx, y + 6, 0x7CFC00);
        float dbm = HandheldAudioClient.signalDbm;
        int bars = dbm < -125 ? 0 : (int) Math.min(10, (dbm + 125) / 6);
        g.drawCenteredString(font, "S " + "|".repeat(bars) + ".".repeat(10 - bars) + (dbm > -199 ? String.format("  %.0f dBm", dbm) : ""), cx, y - 18, 0xFFFFFF);
        g.drawCenteredString(font, "Volume " + s.volume() + "   Squelch " + s.squelch(), cx, y + 84, 0xAAAAAA);
    }

    @Override
    public boolean isPauseScreen() {
        return false;
    }
}
//?}
