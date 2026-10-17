package com.example.evanscomputermod.voice.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.voice.VoiceClientConfig;
import com.mojang.serialization.Codec;
import net.minecraft.client.OptionInstance;
import net.minecraft.client.Options;
import net.minecraft.client.gui.components.OptionsList;
import net.minecraft.client.gui.screens.options.OptionsSubScreen;
import net.minecraft.client.gui.screens.options.SoundOptionsScreen;
import net.minecraft.network.chat.Component;
import net.neoforged.fml.util.ObfuscationReflectionHelper;
import net.neoforged.neoforge.client.event.ScreenEvent;
import net.neoforged.neoforge.common.ModConfigSpec;
import net.neoforged.neoforge.common.NeoForge;
import org.lwjgl.openal.ALC11;
import org.lwjgl.openal.ALUtil;

import java.util.ArrayList;
import java.util.List;
import java.util.Optional;
import java.util.function.Consumer;

/**
 * The microphone settings in Minecraft's own Options > Music & Sounds screen,
 * under the output Device: Microphone input on/off, the input device (the
 * capture devices the system has, listed when the screen opens, like the
 * output Device), input volume 0-200 %, mode and the voice activation level.
 * They are the same values as {@link VoiceClientConfig} (the mod's config file
 * and its Mods > Config screen), saved as they change.
 */
public final class MicrophoneOptions {

    private MicrophoneOptions() {}

    static void register() {
        NeoForge.EVENT_BUS.addListener((ScreenEvent.Init.Post e) -> {
            if (e.getScreen() instanceof SoundOptionsScreen s) add(s);
        });
    }

    private static void add(SoundOptionsScreen screen) {
        OptionsList list;
        try {
            list = ObfuscationReflectionHelper.getPrivateValue(OptionsSubScreen.class, screen, "list");
        } catch (RuntimeException e) {
            return;
        }
        if (list == null) return;
        list.addBig(enabled());
        list.addBig(device());
        list.addSmall(volume(), mode());
        list.addSmall(activation(), indicator());
    }

    private static <T> void save(ModConfigSpec.ConfigValue<T> value, T v) {
        if (!VoiceClientConfig.SPEC.isLoaded()) return;
        value.set(v);
        VoiceClientConfig.SPEC.save();
    }

    static OptionInstance<Boolean> enabled() {
        return OptionInstance.createBoolean("evanscomputermod.options.microphone",
                OptionInstance.cachedConstantTooltip(Component.translatable("evanscomputermod.options.microphone.tooltip")),
                VoiceClientConfig.enabled(), v -> save(VoiceClientConfig.ENABLED, v));
    }

    /** The system's capture devices ("" = the default), or just "" when a voice chat mod provides the microphone. */
    public static List<String> captureDevices() {
        List<String> out = new ArrayList<>();
        out.add("");
        if (VoiceClient.usesVoiceMod()) return out;
        try {
            List<String> found = ALUtil.getStringList(0L, ALC11.ALC_CAPTURE_DEVICE_SPECIFIER);
            if (found != null) for (String d : found) if (d != null && !d.isEmpty() && !out.contains(d)) out.add(d);
        } catch (Throwable ignored) {
            // No OpenAL capture support: only the default.
        }
        String current = VoiceClientConfig.device();
        if (!current.isEmpty() && !out.contains(current)) out.add(current);   // unplugged now; keep it selectable
        return out;
    }

    static OptionInstance<String> device() {
        return new OptionInstance<>("evanscomputermod.options.microphone_device", OptionInstance.noTooltip(),
                (caption, v) -> {
                    if (VoiceClient.usesVoiceMod())
                        return Component.translatable("evanscomputermod.options.microphone_device.voice_mod", VoiceClient.captureName());
                    if (v.isEmpty()) return Component.translatable("options.audioDevice.default");
                    return Component.literal(v.startsWith("OpenAL Soft on ") ? v.substring("OpenAL Soft on ".length()) : v);
                },
                new OptionInstance.LazyEnum<>(MicrophoneOptions::captureDevices, Optional::of, Codec.STRING),
                VoiceClientConfig.device(), v -> save(VoiceClientConfig.DEVICE, v));
    }

    static OptionInstance<Integer> volume() {
        return new OptionInstance<>("evanscomputermod.options.microphone_volume", OptionInstance.noTooltip(),
                (caption, v) -> Options.genericValueLabel(caption, Component.literal(v + "%")),
                new OptionInstance.IntRange(0, 200), VoiceClientConfig.inputVolume(),
                v -> save(VoiceClientConfig.INPUT_VOLUME, v));
    }

    static OptionInstance<VoiceClientConfig.Mode> mode() {
        return new OptionInstance<>("evanscomputermod.options.microphone_mode", OptionInstance.noTooltip(),
                // A cycle button puts the caption in front itself.
                (caption, v) -> Component.translatable("evanscomputermod.options.microphone_mode." + v.name().toLowerCase(java.util.Locale.ROOT)),
                new OptionInstance.Enum<>(List.of(VoiceClientConfig.Mode.values()),
                        Codec.STRING.xmap(VoiceClientConfig.Mode::valueOf, VoiceClientConfig.Mode::name)),
                VoiceClientConfig.mode(), v -> save(VoiceClientConfig.MODE, v));
    }

    static OptionInstance<Integer> activation() {
        return new OptionInstance<>("evanscomputermod.options.microphone_activation", OptionInstance.noTooltip(),
                (caption, v) -> Options.genericValueLabel(caption, Component.literal(v + " dB")),
                new OptionInstance.IntRange(-80, 0), VoiceClientConfig.activationDb(),
                v -> save(VoiceClientConfig.ACTIVATION_DB, v));
    }

    static OptionInstance<Boolean> indicator() {
        Consumer<Boolean> set = v -> save(VoiceClientConfig.SHOW_INDICATOR, v);
        return OptionInstance.createBoolean("evanscomputermod.options.microphone_indicator", VoiceClientConfig.showIndicator(), set);
    }
}
//?}
