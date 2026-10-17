package com.example.evanscomputermod.voice;

//? if <=1.21.1 {
import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Each player's own microphone choices ({@code config/evanscomputermod-voice-client.toml},
 * also under Mods → Evans Computer Mod → Config). The microphone is off until
 * the player turns it on here; it is never opened while it is off.
 */
public final class VoiceClientConfig {

    /** When the microphone sends your voice. */
    public enum Mode {
        /** Only while you hold the Talk key. */
        PUSH_TO_TALK,
        /** Whenever you speak louder than {@code activationLevelDb}, while something is listening. */
        VOICE_ACTIVATION
    }

    public static final ModConfigSpec SPEC;
    public static final ModConfigSpec.BooleanValue ENABLED;
    public static final ModConfigSpec.EnumValue<Mode> MODE;
    public static final ModConfigSpec.IntValue ACTIVATION_DB;
    public static final ModConfigSpec.ConfigValue<String> DEVICE;
    public static final ModConfigSpec.BooleanValue SHOW_INDICATOR;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        ENABLED = b.comment("Microphone input: let Evans Computer Mod hear you through your microphone (Microphone",
                        "blocks, ePhone microphones, handheld radios' push-to-talk). Off: your microphone is never opened.")
                .define("microphoneInput", false);
        MODE = b.comment("PUSH_TO_TALK: you are heard only while you hold the Talk key.",
                        "VOICE_ACTIVATION: you are heard when you speak, while a microphone near you is recording.",
                        "A handheld radio always transmits only while you hold Talk. With Simple Voice Chat or",
                        "Plasmo Voice installed, they decide when your microphone is live and this mod uses their audio.")
                .defineEnum("mode", Mode.PUSH_TO_TALK);
        ACTIVATION_DB = b.comment("Voice activation level, dBFS: louder than this counts as speaking.")
                .defineInRange("activationLevelDb", -40, -80, 0);
        DEVICE = b.comment("Capture device name (empty: the system's default microphone). Ignored when a voice chat mod",
                        "provides the microphone.")
                .define("device", "");
        SHOW_INDICATOR = b.comment("Show a microphone indicator on the HUD while your voice is being sent.")
                .define("showIndicator", true);
        SPEC = b.build();
    }

    private VoiceClientConfig() {}

    public static boolean enabled() {
        try {
            return SPEC.isLoaded() && ENABLED.get();
        } catch (RuntimeException e) {
            return false;
        }
    }

    public static Mode mode() {
        try {
            return SPEC.isLoaded() ? MODE.get() : Mode.PUSH_TO_TALK;
        } catch (RuntimeException e) {
            return Mode.PUSH_TO_TALK;
        }
    }

    public static int activationDb() {
        try {
            return SPEC.isLoaded() ? ACTIVATION_DB.get() : -40;
        } catch (RuntimeException e) {
            return -40;
        }
    }

    public static String device() {
        try {
            return SPEC.isLoaded() ? DEVICE.get() : "";
        } catch (RuntimeException e) {
            return "";
        }
    }

    public static boolean showIndicator() {
        try {
            return !SPEC.isLoaded() || SHOW_INDICATOR.get();
        } catch (RuntimeException e) {
            return true;
        }
    }
}
//?}
