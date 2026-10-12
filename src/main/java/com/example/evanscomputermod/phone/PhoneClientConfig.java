package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * Each player's own ePhone choices ({@code config/evanscomputermod-phone-client.toml}):
 * whether they share their screen with onlookers, and whether they render
 * other players' phone screens at all. Both are sent to the server
 * ({@code PhonePackets.Prefs}) on login and whenever they change (the phone's
 * slot screen has a toggle for each).
 */
public final class PhoneClientConfig {
    public static final ModConfigSpec SPEC;
    public static final ModConfigSpec.BooleanValue SHARE_SCREEN;
    public static final ModConfigSpec.BooleanValue RENDER_OTHERS;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        SHARE_SCREEN = b.comment("Let nearby players see your ePhone's screen (up to the server's visibility.max).")
                .define("shareScreen", true);
        RENDER_OTHERS = b.comment("Draw other players' ePhone screens (off: their phones show dark, and the server",
                "streams you no pixels for them).").define("renderOthers", true);
        SPEC = b.build();
    }

    private PhoneClientConfig() {}

    public static boolean share() {
        try {
            return !SPEC.isLoaded() || SHARE_SCREEN.get();
        } catch (RuntimeException e) {
            return true;
        }
    }

    public static boolean renderOthers() {
        try {
            return !SPEC.isLoaded() || RENDER_OTHERS.get();
        } catch (RuntimeException e) {
            return true;
        }
    }
}
//?}
