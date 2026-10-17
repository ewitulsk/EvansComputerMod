package com.example.evanscomputermod.voice.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import com.example.evanscomputermod.voice.Resampler;
import com.example.evanscomputermod.voice.VoiceClientConfig;
import com.example.evanscomputermod.voice.VoicePackets;
import com.mojang.blaze3d.platform.InputConstants;
import net.minecraft.client.KeyMapping;
import net.minecraft.client.Minecraft;
import net.minecraft.network.chat.Component;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.fml.ModContainer;
import net.neoforged.fml.ModList;
import net.neoforged.neoforge.client.event.ClientPlayerNetworkEvent;
import net.neoforged.neoforge.client.event.ClientTickEvent;
import net.neoforged.neoforge.client.event.RegisterKeyMappingsEvent;
import net.neoforged.neoforge.client.event.RenderGuiEvent;
import net.neoforged.neoforge.client.gui.ConfigurationScreen;
import net.neoforged.neoforge.client.gui.IConfigScreenFactory;
import net.neoforged.neoforge.client.settings.KeyConflictContext;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.network.PacketDistributor;
import org.lwjgl.glfw.GLFW;

import java.util.ArrayList;
import java.util.List;

/**
 * The player's microphone, client side. Nothing is captured unless the player
 * turned on Microphone input ({@link VoiceClientConfig}) and the server says
 * something wants their voice (a recording microphone nearby, or a handheld
 * radio in hand). Then the microphone is opened: this mod's own OpenAL capture,
 * or, when Simple Voice Chat or Plasmo Voice is installed, the audio that mod
 * already captures (the microphone is never opened twice). The voice goes to
 * the server at {@link VoicePackets#VOICE_RATE} as IMA ADPCM while the player
 * holds Talk (push-to-talk) or speaks (voice activation).
 */
public final class VoiceClient {

    public static final String CATEGORY = "key.categories.evanscomputermod.voice";
    public static final KeyMapping TALK = new KeyMapping("key.evanscomputermod.talk", KeyConflictContext.IN_GAME,
            InputConstants.Type.KEYSYM, GLFW.GLFW_KEY_B, CATEGORY);

    /** Voice activation keeps sending this long after the level drops (so words aren't clipped). */
    private static final long HANGOVER_MS = 400;

    private static final OpenAlCapture OWN = new OpenAlCapture();
    private static boolean svc, plasmo;
    private static volatile int demand;
    private static final ImaAdpcm ENCODER = new ImaAdpcm();
    private static Resampler resampler;
    private static int seq;
    private static boolean talkSent;
    private static long lastLoudMs;
    private static long lastSentMs;
    private static long lastHintMs;
    private static long lastTalkMs;
    /** Push-to-talk keeps the microphone open this long after Talk is let go (no re-opening between sentences). */
    private static final long PTT_LINGER_MS = 1500;
    /** Peak of the last captured block, dBFS (HUD level, tests). */
    public static volatile int levelDb = -99;

    private VoiceClient() {}

    public static void register(IEventBus modBus, ModContainer container) {
        svc = ModList.get().isLoaded("voicechat");
        plasmo = ModList.get().isLoaded("plasmovoice");
        if (plasmo) {
            try {
                com.example.evanscomputermod.voice.client.compat.PlasmoVoiceBridge.install();
            } catch (Throwable t) {
                EvansComputerMod.LOGGER.warn("Microphone: couldn't attach to Plasmo Voice ({}); microphones won't hear you", t.toString());
            }
        }
        if (svc || plasmo) EvansComputerMod.LOGGER.info("Microphone: using {}'s capture", svc ? "Simple Voice Chat" : "Plasmo Voice");
        container.registerExtensionPoint(IConfigScreenFactory.class, (mod, parent) -> new ConfigurationScreen(mod, parent));
        modBus.addListener((RegisterKeyMappingsEvent e) -> e.register(TALK));
        NeoForge.EVENT_BUS.addListener((ClientTickEvent.Post e) -> tick());
        NeoForge.EVENT_BUS.addListener((RenderGuiEvent.Post e) -> hud(e));
        NeoForge.EVENT_BUS.addListener((ClientPlayerNetworkEvent.LoggingOut e) -> reset());
    }

    /** The server's {@code DEMAND_*} bits. */
    public static void onDemand(int flags) {
        demand = flags;
    }

    /** Which capture is in use: "Simple Voice Chat", "Plasmo Voice" or "OpenAL". */
    public static String captureName() {
        return svc ? "Simple Voice Chat" : plasmo ? "Plasmo Voice" : "OpenAL";
    }

    private static void reset() {
        demand = 0;
        OWN.stop();
        CaptureBuffer.setAccepting(false);
        talkSent = false;
    }

    private static void tick() {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null || mc.getConnection() == null) {
            if (OWN.running()) reset();
            return;
        }
        boolean enabled = VoiceClientConfig.enabled();
        boolean talk = TALK.isDown() && mc.screen == null;
        long nowMs = System.currentTimeMillis();
        if (talk) lastTalkMs = nowMs;
        // Push-to-talk opens the microphone only around Talk; voice activation (and a voice chat mod's
        // capture, which that mod gates itself) while something listens.
        boolean pttOnly = !svc && VoiceClientConfig.mode() == VoiceClientConfig.Mode.PUSH_TO_TALK;
        boolean wanted = enabled && demand != 0 && (!pttOnly || nowMs - lastTalkMs < PTT_LINGER_MS);
        CaptureBuffer.setAccepting(wanted);
        if (!svc && !plasmo) {
            String device = VoiceClientConfig.device();
            if (wanted && OWN.running() && !device.equals(OWN.openedName())) OWN.stop();   // device changed
            if (wanted && !OWN.running()) OWN.start(device);
            else if (!wanted && OWN.running()) OWN.stop();
        }

        // Everything captured since the last tick, at the voice rate.
        List<short[]> parts = new ArrayList<>();
        int total = 0;
        boolean fromSvc = false;
        int peak = 0;
        CaptureBuffer.Block b;
        while ((b = CaptureBuffer.poll()) != null) {
            if (resampler == null || resampler.inRate() != b.rate()) resampler = new Resampler(b.rate(), VoicePackets.VOICE_RATE);
            for (short s : b.pcm()) peak = Math.max(peak, Math.abs(s));
            short[] r = resampler.process(b.pcm(), 0, b.pcm().length);
            parts.add(r);
            total += r.length;
            fromSvc |= b.source() == CaptureBuffer.Source.SIMPLE_VOICE_CHAT;
        }
        long now = System.currentTimeMillis();
        if (total > 0) {
            levelDb = peak == 0 ? -99 : (int) Math.round(20 * Math.log10(peak / 32768.0));
            if (levelDb > VoiceClientConfig.activationDb()) lastLoudMs = now;
        }

        boolean radio = (demand & VoicePackets.DEMAND_RADIO) != 0;
        boolean open;
        if (fromSvc) open = true;   // Simple Voice Chat only hands over frames while it is sending
        else if (VoiceClientConfig.mode() == VoiceClientConfig.Mode.PUSH_TO_TALK) open = talk;
        else open = talk || now - lastLoudMs < HANGOVER_MS;
        boolean send = wanted && total > 0 && (open || (radio && talk));

        if (send) {
            short[] pcm = new short[total];
            int at = 0;
            for (short[] p : parts) {
                System.arraycopy(p, 0, pcm, at, p.length);
                at += p.length;
            }
            for (int off = 0; off < pcm.length; off += VoicePackets.MAX_SAMPLES) {
                int n = Math.min(VoicePackets.MAX_SAMPLES, pcm.length - off);
                PacketDistributor.sendToServer(new VoicePackets.Voice(seq++, VoicePackets.VOICE_RATE, n, ENCODER.encode(pcm, off, n), talk));
            }
            lastSentMs = now;
            talkSent = talk;
        } else if (talk != talkSent) {
            // Key a radio up or down even when there is no audio to send with it.
            PacketDistributor.sendToServer(new VoicePackets.Talk(talk));
            talkSent = talk;
        }
        if (talk && radio && !enabled && now - lastHintMs > 10_000) {
            lastHintMs = now;
            mc.player.displayClientMessage(Component.translatable("evanscomputermod.voice.mic_off_hint"), true);
        }
    }

    /** Your voice is being sent now. */
    public static boolean live() {
        return System.currentTimeMillis() - lastSentMs < 300;
    }

    private static void hud(RenderGuiEvent.Post e) {
        Minecraft mc = Minecraft.getInstance();
        if (mc.player == null || mc.options.hideGui || !VoiceClientConfig.showIndicator()) return;
        boolean onAir = (demand & VoicePackets.DEMAND_ON_AIR) != 0;
        boolean live = live();
        boolean listening = VoiceClientConfig.enabled() && demand != 0;
        if (!onAir && !live && !listening) return;
        var g = e.getGuiGraphics();
        int x = 6, y = g.guiHeight() - 40;
        String label = onAir ? "TX" : "MIC";
        int dot = live ? 0xFFE03030 : 0xFF707070;
        g.fill(x - 3, y - 3, x + 12 + mc.font.width(label) + 3, y + 11, 0x90000000);
        g.fill(x, y + 1, x + 7, y + 8, dot);
        g.drawString(mc.font, label, x + 11, y, onAir ? 0xFFFF5555 : 0xFFFFFFFF);
    }
}
//?}
