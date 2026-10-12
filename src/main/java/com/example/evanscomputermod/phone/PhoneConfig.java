package com.example.evanscomputermod.phone;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.display.PanelTier;
import net.neoforged.neoforge.common.ModConfigSpec;

/**
 * ePhone server settings ({@code <world>/serverconfig/evanscomputermod-phone-server.toml}).
 * Static getters fall back to the defaults before the file is loaded; tests
 * override single values with {@link #override}.
 *
 * <pre>
 * [visibility]
 *   max = FULL            # OFF | GLOW | FULL: the most other players may see of a phone's screen
 *   mirrorRange = 16      # blocks: how near a player must be to get a FULL mirror
 *   mirrorIntervalTicks = 10   # how often (ticks) a mirror is refreshed for onlookers
 * [battery]
 *   enabled = true
 *   awakePerMinute = 1.0  # % per minute while held (Tier 2 panel)
 *   asleepPerMinute = 0.1 # % per minute while in the hotbar
 *   offPerMinute = 0.0    # % per minute anywhere else (not ticked: phones off are not simulated)
 *   radioPerMinute = 0.3  # % per minute per active radio module (Wi-Fi on, modem attached)
 *   tier1Factor = 0.7, tier2Factor = 1.0, tier3Factor = 1.5   # awake drain scale by panel tier
 *   dockPerMinute = 20.0  # % per minute in a Charging Dock
 *   dockFePerPercent = 0  # FE a dock spends per % (0 = docks charge for free; FE still accepted)
 *   newPhonePercent = 100 # charge of a phone that has never been switched on
 * </pre>
 */
public final class PhoneConfig {

    public static final ModConfigSpec SPEC;

    /** How much of a phone's screen other players can see. */
    public enum Visibility { OFF, GLOW, FULL }

    private static final ModConfigSpec.EnumValue<Visibility> MAX_VISIBILITY;
    private static final ModConfigSpec.IntValue MIRROR_RANGE;
    private static final ModConfigSpec.IntValue MIRROR_INTERVAL;
    private static final ModConfigSpec.BooleanValue BATTERY;
    private static final ModConfigSpec.DoubleValue AWAKE, ASLEEP, OFF, RADIO, T1, T2, T3, DOCK;
    private static final ModConfigSpec.IntValue DOCK_FE, NEW_PERCENT;

    static {
        ModConfigSpec.Builder b = new ModConfigSpec.Builder();
        b.comment("Who sees an ePhone's screen besides its holder (docs/gui/EPHONE.md).").push("visibility");
        MAX_VISIBILITY = b.comment("Most other players may see: OFF (dark screen), GLOW (a lit screen, no pixels sent),",
                "FULL (the screen, streamed to nearby players who opted in; the holder must share too).")
                .defineEnum("max", Visibility.FULL);
        MIRROR_RANGE = b.comment("FULL: blocks within which onlookers get the mirror.").defineInRange("mirrorRange", 16, 2, 64);
        MIRROR_INTERVAL = b.comment("FULL: ticks between mirror updates for onlookers (the holder is never throttled).")
                .defineInRange("mirrorIntervalTicks", 10, 1, 200);
        b.pop();
        b.comment("ePhone battery. Percent per minute of real (server) time.").push("battery");
        BATTERY = b.comment("Phones have a battery; false = they never run down.").define("enabled", true);
        AWAKE = b.comment("Drain while held (awake), with a Tier 2 panel.").defineInRange("awakePerMinute", 1.0, 0, 10_000);
        ASLEEP = b.comment("Drain while in the hotbar (asleep).").defineInRange("asleepPerMinute", 0.1, 0, 10_000);
        OFF = b.comment("Drain anywhere else (off). Off phones are paused, so only an attached phone in its",
                "last ticks pays this; keep it near 0.").defineInRange("offPerMinute", 0.0, 0, 10_000);
        RADIO = b.comment("Extra drain per active radio module (Wi-Fi on, modem attached to a tower), awake or asleep.")
                .defineInRange("radioPerMinute", 0.3, 0, 10_000);
        T1 = b.comment("Awake drain factor with a Tier 1 panel.").defineInRange("tier1Factor", 0.7, 0, 100);
        T2 = b.comment("Awake drain factor with a Tier 2 panel.").defineInRange("tier2Factor", 1.0, 0, 100);
        T3 = b.comment("Awake drain factor with a Tier 3 panel.").defineInRange("tier3Factor", 1.5, 0, 100);
        DOCK = b.comment("Charge rate in a Charging Dock.").defineInRange("dockPerMinute", 20.0, 0, 100_000);
        DOCK_FE = b.comment("FE a Charging Dock spends per percent. 0 = docks charge for free (FE is still accepted",
                "and stored); above 0 a dock only charges from the FE it was given.").defineInRange("dockFePerPercent", 0, 0, 1_000_000);
        NEW_PERCENT = b.comment("Charge of a brand-new phone.").defineInRange("newPhonePercent", 100, 0, 100);
        b.pop();
        SPEC = b.build();
    }

    private PhoneConfig() {}

    private static <T> T get(ModConfigSpec.ConfigValue<T> v, T fallback) {
        Object o = OVERRIDES.get(v);
        if (o != null) {
            @SuppressWarnings("unchecked") T t = (T) o;
            return t;
        }
        try {
            return SPEC.isLoaded() ? v.get() : fallback;
        } catch (RuntimeException e) {
            return fallback;
        }
    }

    private static final java.util.Map<Object, Object> OVERRIDES = new java.util.concurrent.ConcurrentHashMap<>();

    /** Test hook: force a value by its key ({@code visibility.max}, {@code battery.awakePerMinute}, ...); null clears. */
    public static void override(String key, Object value) {
        ModConfigSpec.ConfigValue<?> v = switch (key) {
            case "visibility.max" -> MAX_VISIBILITY;
            case "visibility.mirrorRange" -> MIRROR_RANGE;
            case "visibility.mirrorIntervalTicks" -> MIRROR_INTERVAL;
            case "battery.enabled" -> BATTERY;
            case "battery.awakePerMinute" -> AWAKE;
            case "battery.asleepPerMinute" -> ASLEEP;
            case "battery.offPerMinute" -> OFF;
            case "battery.radioPerMinute" -> RADIO;
            case "battery.dockPerMinute" -> DOCK;
            case "battery.dockFePerPercent" -> DOCK_FE;
            case "battery.newPhonePercent" -> NEW_PERCENT;
            default -> throw new IllegalArgumentException("unknown phone config key " + key);
        };
        if (value == null) OVERRIDES.remove(v);
        else OVERRIDES.put(v, value);
    }

    /** Clear every test override. */
    public static void clearOverrides() {
        OVERRIDES.clear();
    }

    public static Visibility maxVisibility() { return get(MAX_VISIBILITY, Visibility.FULL); }
    public static int mirrorRange() { return get(MIRROR_RANGE, 16); }
    public static int mirrorIntervalTicks() { return get(MIRROR_INTERVAL, 10); }
    public static boolean batteryEnabled() { return get(BATTERY, true); }
    public static double awakePerMinute() { return get(AWAKE, 1.0); }
    public static double asleepPerMinute() { return get(ASLEEP, 0.1); }
    public static double offPerMinute() { return get(OFF, 0.0); }
    public static double radioPerMinute() { return get(RADIO, 0.3); }
    public static double dockPerMinute() { return get(DOCK, 20.0); }
    public static int dockFePerPercent() { return get(DOCK_FE, 0); }
    public static int newPhonePercent() { return get(NEW_PERCENT, 100); }

    public static double tierFactor(PanelTier tier) {
        if (tier == null) return 0.5;   // headless: no backlight
        return switch (tier) {
            case PHOSPHOR -> get(T1, 0.7);
            case COLOR_LCD -> get(T2, 1.0);
            case HD -> get(T3, 1.5);
        };
    }
}
//?}
