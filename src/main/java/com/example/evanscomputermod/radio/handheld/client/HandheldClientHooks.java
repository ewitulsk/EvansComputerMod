package com.example.evanscomputermod.radio.handheld.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.handheld.HandheldRadioItem;
import com.example.evanscomputermod.radio.handheld.HandheldSettings;
import com.example.evanscomputermod.speaker.client.HandheldAudioClient;
import net.minecraft.client.Minecraft;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RenderGuiEvent;
import net.neoforged.neoforge.common.NeoForge;

/** Client-only: stop stale audio, and a small HUD (band, frequency, S-meter) while a switched-on radio is held. */
public final class HandheldClientHooks {
    private HandheldClientHooks() {}

    public static void register() {
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> HandheldAudioClient.tick());
        NeoForge.EVENT_BUS.addListener((RenderGuiEvent.Post e) -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.player == null || mc.options.hideGui || mc.screen != null) return;
            ItemStack stack = ItemStack.EMPTY;
            for (InteractionHand h : InteractionHand.values())
                if (mc.player.getItemInHand(h).getItem() instanceof HandheldRadioItem) stack = mc.player.getItemInHand(h);
            if (stack.isEmpty()) return;
            HandheldSettings s = HandheldSettings.read(stack);
            if (!s.on()) return;
            float dbm = HandheldAudioClient.signalDbm;
            int bars = dbm < -125 ? 0 : (int) Math.min(10, (dbm + 125) / 6);
            var g = e.getGuiGraphics();
            int x = g.guiWidth() - 118, y = g.guiHeight() / 2 + 20;
            g.fill(x - 4, y - 4, x + 114, y + 22, 0x90000000);
            g.drawString(mc.font, s.band() + " " + HandheldRadioItem.freqLabel(s.freqHz()), x, y, 0x7CFC00);
            g.drawString(mc.font, "S " + "|".repeat(bars) + ".".repeat(10 - bars), x, y + 10, 0xFFFFFF);
        });
    }
}
//?}
