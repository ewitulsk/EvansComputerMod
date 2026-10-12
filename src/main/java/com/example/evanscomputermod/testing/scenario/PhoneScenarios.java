package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ComputerStorage;
import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralValues;
import com.example.evanscomputermod.network.ComputerTargets;
import com.example.evanscomputermod.phone.ChargingDockBlock;
import com.example.evanscomputermod.phone.ChargingDockBlockEntity;
import com.example.evanscomputermod.phone.PhoneConfig;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhonePackets;
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.radio.wifi.ap.AccessPointBlockEntity;
import com.example.evanscomputermod.radio.wifi.ap.ApPackets;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;
import net.neoforged.neoforge.common.util.FakePlayer;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.UUID;
import java.util.WeakHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * ePhone scenarios (docs/gui/EPHONE.md), operated by scenario players: the
 * phone is used, carried, held, put in the hotbar and away, docked and walked
 * around as a person would, through the same calls the game makes (first
 * use through {@code Item.use}, hotkeys through the packet handler's
 * {@code PhoneServer.deliverHotkey}, typing through the UUID input path with
 * its {@code acceptsInput} check). The same definitions run as GameTests
 * ({@code ecm_phone}) and from {@code /ecm scenario spawn phone_*}; in a
 * normal world the holder is the scenario's builder (an unseen helper player),
 * so watch the chat lines; {@code /ecm scenario commands phone_basics} says how
 * to do the same with your own phone.
 *
 * <ul>
 *   <li>{@code phone_basics}: held = awake (boots, console 30x25), a counter
 *       program runs; hotkey slot 1 reaches the computer as {@code phone_hotkey}
 *       events (control: refused while the phone is only in the hotbar); hotbar =
 *       asleep, the counter freezes; back in hand it continues; into the
 *       inventory = off (detached, same computer paused, counter frozen); back
 *       in the hotbar it resumes the same computer where it was; off hand =
 *       awake.</li>
 *   <li>{@code phone_battery}: the battery drains while held, a Charging Dock
 *       charges a docked phone (its lamp lit), an empty battery powers the
 *       phone off (the computer stops; control: an empty phone in hand stays
 *       off) and it cold-boots once charged.</li>
 *   <li>{@code phone_visibility}: server max FULL streams the mirror to an
 *       onlooker; GLOW sends an onlooker no display packets at all (they get the
 *       state only); the holder's "share" off and the onlooker's "render others"
 *       off do the same at FULL.</li>
 *   <li>{@code phone_duplicates}: two players hold copies of one phone: the
 *       first holder keeps it (no flapping), the second takes over when the first
 *       puts it away; a dimension change moves the phone's radios; dropping it
 *       turns it off.</li>
 *   <li>{@code phone_wifi_walk}: the phone joins an open Access Point and pings
 *       a gateway computer through it while its holder walks 28 blocks away
 *       (the AP sees the phone's signal fall); behind a stone wall, where
 *       {@code /ecm radio link} puts the signal under the module's -92 dBm
 *       sensitivity, the pings stop.</li>
 * </ul>
 */
public final class PhoneScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    /** Where the holder stands (relative). */
    static final BlockPos STAND = new BlockPos(0, 1, 0);
    static final String COUNTER = "import time\ni = 0\nwhile True:\n    i += 1\n    print('count', i)\n    time.sleep(0.25)\n";
    private static final Pattern COUNT = Pattern.compile("count (\\d+)");

    /** Per run: the phone, the computer seen before a change, the second player... */
    static final class State {
        UUID phone;
        ComputerInstance computer;
        int count = -1;
        long since;
        ScenarioPlayer other;
        PeripheralEventBus.Subscriber events;
        final List<String> heard = new ArrayList<>();
        int packets;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private PhoneScenarios() {}

    // ------------------------------------------------------------ the player's hands

    static FakePlayer holder(ScenarioRun r) {
        return r.player().entity();
    }

    /** Stand at {@code rel} (eyes 1.62 up), looking along +x. */
    static void standAt(ScenarioPlayer p, ScenarioRun r, Vec3 rel) {
        Vec3 feet = Vec3.atLowerCornerOf(r.origin()).add(rel);
        Vec3 eye = feet.add(0, 1.62, 0);
        p.lookFrom(eye, eye.add(1, 0, 0));
    }

    /**
     * A new ePhone in hotbar slot 1 (selected), right-clicked once in the air:
     * its first use gives it its computer.
     */
    static void givePhone(ScenarioRun r) {
        PhoneConfig.clearOverrides();
        FakePlayer p = holder(r);
        standAt(r.player(), r, Vec3.atLowerCornerOf(STAND).add(0.5, 0, 0.5));
        PhoneServer.track(p);
        ItemStack phone = new ItemStack(PhoneContent.EPHONE.get());
        r.player().useInAir(phone, false);
        UUID id = PhoneItemData.id(phone);
        if (id == null) throw new IllegalStateException("first use gave the phone no computer");
        p.getInventory().setItem(0, phone);
        p.getInventory().selected = 0;
        st(r).phone = id;
        r.say("§7  " + p.getGameProfile().getName() + " used a new ePhone (computer " + id.toString().substring(0, 8)
                + ") and holds it (hotbar slot 1)");
    }

    static PhoneHost host(ScenarioRun r) {
        UUID id = st(r).phone;
        return id == null ? null : PhoneServer.find(r.level().getServer(), id);
    }

    /** Inventory slot of the phone on {@code p} (0..35; 40 = off hand), or -1. */
    static int slotOf(FakePlayer p, UUID id) {
        Inventory inv = p.getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) if (id.equals(PhoneItemData.id(inv.getItem(i)))) return i;
        return -1;
    }

    /** Move the phone to inventory slot {@code to} (0-8 hotbar, 9-35 inventory, 40 off hand) as a player would. */
    static void move(ScenarioRun r, FakePlayer p, int to) {
        UUID id = st(r).phone;
        int from = slotOf(p, id);
        if (from < 0) throw new IllegalStateException(p.getGameProfile().getName() + " has no phone to move");
        Inventory inv = p.getInventory();
        ItemStack s = inv.getItem(from);
        inv.setItem(from, inv.getItem(to));
        inv.setItem(to, s);
    }

    static void select(FakePlayer p, int slot) {
        p.getInventory().selected = slot;
    }

    /**
     * Type on the phone's console as its holder: the UUID input path (acceptsInput:
     * the holder of an awake phone only), into the kernel's console, which keeps
     * running behind the phone's GUI (P5b; there is no console screen on a phone,
     * so this is the scenarios' harness for running programs, as GuiKernelTest does).
     */
    static boolean type(ScenarioRun r, FakePlayer p, String text) {
        IComputerHost h = ComputerTargets.resolveInput(p, st(r).phone, IComputerHost.DISPLAY_OWN);
        ComputerInstance c = h == null ? null : ComputerTargets.instance(h);
        if (c == null) return false;
        c.consoleInput(text);
        return true;
    }

    static String screen(ScenarioRun r) {
        PhoneHost h = host(r);
        return h == null ? "" : ScenarioRun.screen(h.display());
    }

    /** Highest "count N" on the phone's screen, or -1. */
    static int count(ScenarioRun r) {
        Matcher m = COUNT.matcher(screen(r));
        int best = -1;
        while (m.find()) best = Math.max(best, Integer.parseInt(m.group(1)));
        return best;
    }

    static void writeFile(ScenarioRun r, String name, String content) {
        try {
            Path dir = ComputerStorage.path(r.level().getServer(), st(r).phone);
            Files.createDirectories(dir);
            Files.writeString(dir.resolve(name), content);
        } catch (java.io.IOException e) {
            throw new IllegalStateException("can't write " + name + " on the phone: " + e);
        }
    }

    /** Null once the phone is awake, booted to the shell and its console follows a Tier 2 portrait panel. */
    static String booted(ScenarioRun r) {
        PhoneHost h = host(r);
        if (h == null) return "no phone host yet";
        if (h.state() != PhoneState.AWAKE || !h.isAttached()) return "phone is " + h.state() + (h.isAttached() ? "" : " (detached)");
        if (h.instance() == null) return "booting";
        if (!screen(r).contains("Welcome to Terminal OS")) return "waiting for the shell";
        var d = h.display();
        if (d.getWidth() != 30 || d.getHeight() != 25) return "console is " + d.getWidth() + "x" + d.getHeight();
        // P5b: a phone is always in GUI mode, with the phone shell.
        var ui = h.instance().getUiManager();
        if (ui.phone() == null || ui.compositor() == null || !ui.compositor().shellConnected()) return "waiting for the phone shell";
        return null;
    }

    /** Type a shell line on the phone (after clearing the screen) and wait for {@code re}; {@code fail} ends the run at once. */
    static void phoneRun(Scenario.Builder b, String line, String re, String fail, String what, int timeoutMs) {
        Pattern want = Pattern.compile(re, Pattern.MULTILINE), bad = fail == null ? null : Pattern.compile(fail, Pattern.MULTILINE);
        clearScreen(b);
        b.mutate(r -> {
            if (!type(r, holder(r), line + "\n")) r.fail("the phone refused its holder's typing");
        }, "on the phone: " + line);
        b.await(r -> {
            String s = screen(r);
            if (bad != null && bad.matcher(s).find()) {
                r.fail(what + ": the phone printed\n" + s);
                return null;
            }
            return want.matcher(s).find() ? null : "phone screen:\n" + s;
        }, what, timeoutMs);
    }

    /** Type {@code clear} on the phone and wait until only the prompt is left (typing ahead of a running program is lost). */
    static void clearScreen(Scenario.Builder b) {
        b.mutate(r -> type(r, holder(r), "clear\n"), "on the phone: clear");
        b.await(r -> screen(r).strip().equals("/ >") ? null : "screen:\n" + screen(r), "the phone's screen is clear", 10_000);
    }

    // ------------------------------------------------------------ phone_basics

    static List<BlockPos> floor(int x0, int z0, int x1, int z1) {
        return PlayerKit.box(x0, 0, z0, x1, 0, z1);
    }

    static Scenario.Decor stage(List<BlockPos> floor, java.util.function.Consumer<ScenarioRun> build) {
        return PlayerKit.decor(floor, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), build,
                PhoneScenarios::cleanup);
    }

    /** Undo a phone scenario: test overrides, tracked players, the phone and its computer. */
    static void cleanup(ScenarioRun r) {
        PhoneConfig.clearOverrides();
        State s = STATES.remove(r);
        PhoneServer.untrack(holder(r));
        if (s == null) return;
        if (s.other != null) PhoneServer.untrack(s.other.entity());
        if (s.phone == null) return;
        Inventory inv = holder(r).getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) if (s.phone.equals(PhoneItemData.id(inv.getItem(i)))) inv.setItem(i, ItemStack.EMPTY);
        PhoneHost h = PhoneServer.find(r.level().getServer(), s.phone);
        if (h != null && h.instance() != null) {
            h.instance().close();
            com.example.evanscomputermod.computer.ComputerHost.get(r.level().getServer(), s.phone).forget();
        }
    }

    private static Scenario basics() {
        var b = Scenario.builder("phone_basics",
                        "ePhone states: held = awake, hotbar = asleep (frozen), elsewhere = off (paused) and back where it was;"
                                + " hotkey slot 1 reaches the computer")
                .asPlayer()
                .decor(stage(floor(-2, -2, 2, 2), PhoneScenarios::givePhone))
                .note("Setup (done for you): a player right-clicks a new ePhone (its first use gives it a computer) and holds it."
                        + " By hand: craft or take an ePhone, hold it, wait for the shell.")
                .await(PhoneScenarios::booted, "held: the phone boots awake, its console 30x25 (Tier 2 portrait panel)", 40_000)
                .note("A counter program: python count.py (prints count 1, 2, 3... every 0.25 s)")
                .mutate(r -> {
                    writeFile(r, "count.py", COUNTER);
                    if (!type(r, holder(r), "python count.py\n")) r.fail("the phone refused its holder's typing");
                }, "on the phone: python count.py")
                .await(r -> count(r) >= 3 ? null : "count " + count(r) + "\n" + screen(r), "the counter runs (count 3)", 30_000)
                .note("Hotkey slot 1 (Controls > ePhone slot 1) pressed and released while held: the phone takes it"
                        + " (it goes to the open app; phone_gui shows that)")
                .mutate(r -> {
                    State s = st(r);
                    for (boolean down : new boolean[]{true, false}) {
                        if (!PhoneServer.deliverHotkey(holder(r), new PhonePackets.Hotkey(s.phone, 1, down)))
                            r.fail("the phone refused its holder's hotkey");
                    }
                }, "press and release hotkey slot 1")
                .note("Asleep: select another hotbar slot (the phone stays in the hotbar)")
                .mutate(r -> select(holder(r), 1), "select hotbar slot 2")
                .await(r -> asleepFrozen(r, PhoneState.ASLEEP, true), "asleep: the computer sleeps (kernel and programs stopped)", 5_000)
                .mutate(r -> {
                    if (PhoneServer.deliverHotkey(holder(r), new PhonePackets.Hotkey(st(r).phone, 1, true)))
                        r.fail("a hotkey was taken while the phone was only in the hotbar");
                }, "control: a hotkey with the phone not in hand is refused")
                .waitMs(1_500, "asleep for 1.5 s")
                .await(r -> unchanged(r), "the counter did not move while asleep", 1_000)
                .mutate(r -> select(holder(r), 0), "select the phone again")
                .await(r -> resumed(r), "awake again: the same computer continues counting", 10_000)
                .note("Off: move the phone into the inventory (out of the hotbar)")
                .mutate(r -> move(r, holder(r), 9), "move the phone to an inventory slot")
                .await(r -> asleepFrozen(r, PhoneState.OFF, false), "off: paused in memory, host detached", 5_000)
                .waitMs(1_500, "off for 1.5 s")
                .await(r -> unchanged(r), "the counter did not move while off", 1_000)
                .mutate(r -> {
                    move(r, holder(r), 0);
                    select(holder(r), 0);
                }, "put the phone back in hotbar slot 1 and hold it")
                .await(r -> resumed(r), "it resumes where it was: same computer, the counter goes on from its value", 10_000)
                .note("Off hand: hold it in the off hand with an empty main hand")
                .mutate(r -> {
                    move(r, holder(r), 40);
                    select(holder(r), 2);
                }, "move the phone to the off hand")
                .await(r -> {
                    PhoneHost h = host(r);
                    return h.state() == PhoneState.AWAKE && h.isAttached() ? resumed(r) : "phone is " + h.state();
                }, "off hand: awake", 10_000)
                .mutate(r -> type(r, holder(r), "\u0014"), "Ctrl+T stops the counter")
                .timeLimit(60_000);
        return b.build();
    }

    private static String hotkeyEvents(ScenarioRun r) {
        State s = st(r);
        try {
            PeripheralEventBus.Event e;
            while ((e = PeripheralEventBus.await(s.events, null, 0)) != null) {
                s.heard.add(e.name() + PeripheralValues.decode(e.encoded()));
            }
        } catch (Exception ex) {
            return ex.toString();
        }
        String want = "[phone_hotkey, phone, 1, true]";
        List<String> hot = s.heard.stream().filter(h -> h.startsWith("phone_hotkey")).toList();
        if (hot.size() >= 2 && hot.get(0).equals("phone_hotkey" + want) && hot.get(1).equals("phone_hotkey[phone_hotkey, phone, 1, false]")) {
            host(r).instance().getPeripheralEvents().unsubscribe(s.events);
            return null;
        }
        return "events so far: " + s.heard;
    }

    /** Null once the phone is in {@code state}, its computer (the same one) has stopped, and remember the counter. */
    private static String asleepFrozen(ScenarioRun r, PhoneState state, boolean attached) {
        PhoneHost h = host(r);
        State s = st(r);
        if (h.state() != state) return "phone is " + h.state();
        if (h.isAttached() != attached) return attached ? "detached" : "still attached";
        ComputerInstance c = h.instance();
        if (c == null) return "the computer stopped (it should be paused)";
        if (s.computer == null) s.computer = c;
        if (c != s.computer) return "a different computer: it was rebooted";
        if (!c.isAsleep()) return "the computer is not asleep";
        try {
            if (!c.awaitAsleep(0)) return "the sleep has not landed";
        } catch (InterruptedException e) {
            return "interrupted";
        }
        s.count = count(r);
        return null;
    }

    private static String unchanged(ScenarioRun r) {
        State s = st(r);
        int now = count(r);
        if (now != s.count) {
            r.fail("the counter moved from " + s.count + " to " + now + " while the phone was " + host(r).state());
            return null;
        }
        return host(r).instance() == s.computer ? null : "the computer changed";
    }

    private static String resumed(ScenarioRun r) {
        PhoneHost h = host(r);
        State s = st(r);
        if (h.state() != PhoneState.AWAKE) return "phone is " + h.state();
        if (h.instance() != s.computer && s.computer != null) {
            r.fail("the phone came back with a different computer (it was rebooted)");
            return null;
        }
        int now = count(r);
        if (now <= s.count) return "counter still at " + now;
        if (now > s.count + 40) {
            r.fail("the counter jumped from " + s.count + " to " + now);
            return null;
        }
        if (screen(r).contains("Welcome to Terminal OS") && now < s.count) return "rebooted";
        s.count = now;
        return null;
    }

    // ------------------------------------------------------------ phone_battery

    static final BlockPos DOCK = new BlockPos(2, 1, 0);

    private static Scenario battery() {
        return Scenario.builder("phone_battery",
                        "the battery drains while held, a Charging Dock charges it, an empty battery powers the phone off")
                .asPlayer()
                .realTime()
                .decor(stage(floor(-2, -2, 4, 2), r -> {
                    // The dock first: placing it uses the player's hand.
                    r.player().place(PhoneContent.CHARGING_DOCK_ITEM.get(), r.abs(DOCK), Direction.WEST,
                            s -> s.getValue(ChargingDockBlock.FACING) == Direction.WEST);
                    givePhone(r);
                    // Fast rates so the run stays short (a real server: 1 %/min held, 20 %/min docked).
                    PhoneConfig.override("battery.awakePerMinute", 300.0);
                    PhoneConfig.override("battery.dockPerMinute", 600.0);
                }))
                .note("Setup (done for you): a new phone held (100 %), a Charging Dock placed beside the player."
                        + " Drain and charge rates are raised for the run (battery.awakePerMinute 300, dockPerMinute 600).")
                .await(PhoneScenarios::booted, "the phone boots", 40_000)
                .await(r -> battery(r) < 95 && stackBattery(r) < 96 ? null
                        : String.format(Locale.ROOT, "battery %.1f%% (stack %.1f%%)", battery(r), stackBattery(r)),
                        "held, the battery drains (below 95 %, written to the item)", 20_000)
                .mutate(r -> r.player().use(holder(r).getInventory().getItem(0), r.abs(DOCK), Direction.UP, false),
                        "right-click the dock with the phone: docked")
                .await(r -> {
                    ChargingDockBlockEntity be = dock(r);
                    if (be == null || be.phoneStack().isEmpty()) return "the dock has no phone";
                    if (host(r).state() != PhoneState.OFF) return "phone is " + host(r).state();
                    if (!r.level().getBlockState(r.abs(DOCK)).getValue(ChargingDockBlock.ACTIVE)) return "the dock lamp is off";
                    st(r).count = (int) Math.round(PhoneItemData.batteryPercent(be.phoneStack()) * 100);
                    return null;
                }, "docked: the phone is off and the dock's lamp is lit", 5_000)
                .await(r -> PhoneItemData.batteryPercent(dock(r).phoneStack()) * 100 >= st(r).count + 300 ? null
                        : String.format(Locale.ROOT, "docked phone at %.1f%%", PhoneItemData.batteryPercent(dock(r).phoneStack())),
                        "the dock charges it (3 % up)", 20_000)
                .mutate(r -> takeBack(r), "take the phone back (empty hand)")
                .await(r -> {
                    int slot = slotOf(holder(r), st(r).phone);
                    if (slot < 0) return "the player has no phone";
                    move(r, holder(r), 0);
                    select(holder(r), 0);
                    return host(r).state() == PhoneState.AWAKE ? null : "phone is " + host(r).state();
                }, "back in hand, awake", 5_000)
                .note("An empty battery powers the phone off")
                .mutate(r -> {
                    st(r).computer = host(r).instance();
                    host(r).setBatteryForTest(0.5);
                }, "run the battery almost flat")
                .await(r -> {
                    PhoneHost h = host(r);
                    if (h.state() != PhoneState.OFF) return "phone is " + h.state() + String.format(Locale.ROOT, " at %.2f%%", h.batteryPercent());
                    if (h.instance() != null) return "the computer still exists";
                    return h.transitions().stream().anyMatch(t -> t.contains("battery empty")) ? null : "transitions " + h.transitions();
                }, "flat: the phone powers off and its computer stops", 10_000)
                .waitMs(1_000, "control: held with an empty battery")
                .await(r -> host(r).instance() == null && !host(r).isBooting() && host(r).state() == PhoneState.OFF ? null
                        : "an empty phone started again", "an empty phone in hand stays off", 1_000)
                .mutate(r -> {
                    PhoneConfig.override("battery.awakePerMinute", 1.0);
                    r.player().use(holder(r).getInventory().getItem(0), r.abs(DOCK), Direction.UP, false);
                }, "dock it again")
                .await(r -> PhoneItemData.batteryPercent(dock(r).phoneStack()) >= 5 ? null : "charging",
                        "it charges back up (5 %)", 20_000)
                .mutate(r -> {
                    takeBack(r);
                    move(r, holder(r), 0);
                    select(holder(r), 0);
                }, "take it and hold it")
                .await(r -> {
                    String b2 = booted(r);
                    if (b2 != null) return b2;
                    return host(r).instance() != st(r).computer ? null : "same computer?";
                }, "charged, it cold-boots (a new computer)", 40_000)
                .timeLimit(60_000)
                .build();
    }

    /** Right-click the dock with an empty hand (hotbar slot 6 selected; the phone lands in the first free slot). */
    static void takeBack(ScenarioRun r) {
        select(holder(r), 5);
        r.player().useEmpty(r.abs(DOCK), Direction.UP, false);
    }

    static double battery(ScenarioRun r) {
        return host(r).batteryPercent();
    }

    static double stackBattery(ScenarioRun r) {
        int slot = slotOf(holder(r), st(r).phone);
        return slot < 0 ? -1 : PhoneItemData.batteryPercent(holder(r).getInventory().getItem(slot));
    }

    static ChargingDockBlockEntity dock(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(DOCK)) instanceof ChargingDockBlockEntity be ? be : null;
    }

    // ------------------------------------------------------------ phone_visibility

    private static Scenario visibility() {
        return Scenario.builder("phone_visibility",
                        "server visibility GLOW sends onlookers no display packets; FULL streams the mirror; share and render-others toggles")
                .asPlayer()
                .decor(stage(floor(-2, -2, 6, 2), PhoneScenarios::givePhone))
                .note("Setup (done for you): a player holds a phone; the screen keeps changing (the counter program).")
                .await(PhoneScenarios::booted, "the phone boots", 40_000)
                .mutate(r -> {
                    writeFile(r, "count.py", COUNTER);
                    type(r, holder(r), "python count.py\n");
                }, "on the phone: python count.py")
                .await(r -> count(r) >= 2 ? null : "waiting for the counter", "the screen changes every 0.25 s", 30_000)
                .note("Server visibility.max = GLOW; an onlooker walks up (3 blocks away, renders others' phones)")
                .mutate(r -> {
                    PhoneConfig.override("visibility.max", PhoneConfig.Visibility.GLOW);
                    State s = st(r);
                    s.other = new ScenarioPlayer(r.level(), "Onlooker");
                    standAt(s.other, r, new Vec3(3.5, 0, 0.5));
                    PhoneServer.setPrefs(s.other.entity().getUUID(), PhonePackets.Prefs.DEFAULT);
                    PhoneServer.track(s.other.entity());
                    s.since = System.currentTimeMillis();
                }, "GLOW; the onlooker arrives")
                .await(r -> {
                    PhoneHost h = host(r);
                    if (h.visibility() != PhoneConfig.Visibility.GLOW) return "visibility " + h.visibility();
                    return System.currentTimeMillis() - st(r).since > 2_000 ? null : "watching";
                }, "for 2 s the phone shows GLOW to onlookers", 5_000)
                .await(r -> {
                    if (onlookerPackets(r) != 0 || !host(r).mirrorViewers().isEmpty())
                        r.fail("GLOW sent the onlooker " + onlookerPackets(r) + " display packets");
                    return null;
                }, "GLOW: no display packets reached the onlooker", 1_000)
                .note("Control: visibility.max = FULL streams the mirror to the onlooker")
                .mutate(r -> PhoneConfig.override("visibility.max", PhoneConfig.Visibility.FULL), "FULL")
                .await(r -> host(r).mirrorViewers().contains(st(r).other.entity().getUUID()) && onlookerPackets(r) > 0 ? null
                        : "mirror " + host(r).mirrorViewers() + ", packets " + onlookerPackets(r),
                        "FULL: the onlooker is in the mirror and got display packets", 5_000)
                .note("The holder turns 'share my screen' off: onlookers get GLOW again")
                .mutate(r -> {
                    PhoneServer.setPrefs(holder(r).getUUID(), new PhonePackets.Prefs(false, true));
                }, "holder: share off")
                .await(r -> host(r).visibility() == PhoneConfig.Visibility.GLOW && host(r).mirrorViewers().isEmpty() ? null
                        : "visibility " + host(r).visibility(), "share off: GLOW, no mirror", 5_000)
                .mutate(r -> {
                    PhoneServer.setPrefs(holder(r).getUUID(), PhonePackets.Prefs.DEFAULT);
                    PhoneServer.setPrefs(st(r).other.entity().getUUID(), new PhonePackets.Prefs(true, false));
                }, "holder shares again; the onlooker turns 'render other players' phones' off")
                .await(r -> host(r).visibility() == PhoneConfig.Visibility.FULL && host(r).mirrorViewers().isEmpty() ? null
                        : "visibility " + host(r).visibility() + ", mirror " + host(r).mirrorViewers(),
                        "render-others off: the onlooker leaves the mirror", 5_000)
                .mutate(r -> type(r, holder(r), "\u0014"), "Ctrl+T stops the counter")
                .timeLimit(60_000)
                .build();
    }

    private static int onlookerPackets(ScenarioRun r) {
        State s = st(r);
        return s.other == null ? 0 : host(r).packetsSentTo(s.other.entity().getUUID());
    }

    // ------------------------------------------------------------ phone_duplicates

    private static Scenario duplicates() {
        return Scenario.builder("phone_duplicates",
                        "two players hold copies of one phone (no flapping, hand-over), a dimension change, dropping it")
                .asPlayer()
                .decor(stage(floor(-2, -2, 6, 2), PhoneScenarios::givePhone))
                .await(PhoneScenarios::booted, "the phone boots", 40_000)
                .note("A second player holds a copy of the same phone")
                .mutate(r -> {
                    State s = st(r);
                    s.computer = host(r).instance();
                    s.other = new ScenarioPlayer(r.level(), "Twin");
                    standAt(s.other, r, new Vec3(3.5, 0, 0.5));
                    PhoneServer.track(s.other.entity());
                    s.other.entity().getInventory().setItem(0, holder(r).getInventory().getItem(0).copy());
                    s.other.entity().getInventory().selected = 0;
                    s.packets = host(r).transitions().size();
                    s.since = System.currentTimeMillis();
                }, "the copy goes in the second player's hand")
                .await(r -> {
                    PhoneHost h = host(r);
                    if (!holder(r).getUUID().equals(h.holderId())) {
                        r.fail("the phone moved to " + h.holderName() + " while its first holder still held it");
                        return null;
                    }
                    if (h.transitions().size() != st(r).packets) {
                        r.fail("state changed while both held it: " + h.transitions());
                        return null;
                    }
                    return System.currentTimeMillis() - st(r).since > 1_500 ? null : "watching";
                }, "for 1.5 s the first holder keeps it (no flapping)", 5_000)
                .mutate(r -> move(r, holder(r), 9), "the first player puts their copy away")
                .await(r -> {
                    PhoneHost h = host(r);
                    if (!st(r).other.entity().getUUID().equals(h.holderId())) return "holder " + h.holderName();
                    if (h.state() != PhoneState.AWAKE) return "phone is " + h.state();
                    return h.instance() == st(r).computer ? null : "a different computer";
                }, "the second player now holds it, awake, the same computer", 5_000)
                .note("The second player goes to the Nether: the phone's radios follow")
                .mutate(r -> {
                    ServerLevel nether = r.level().getServer().getLevel(Level.NETHER);
                    if (nether == null) {
                        r.say("§7  (no Nether in this world: skipped)");
                        return;
                    }
                    FakePlayer t = st(r).other.entity();
                    t.setServerLevel(nether);
                    t.moveTo(0.5, 100, 0.5);
                }, "change the second player's dimension")
                .await(r -> {
                    PhoneHost h = host(r);
                    if (r.level().getServer().getLevel(Level.NETHER) == null) return null;
                    if (h.state() != PhoneState.AWAKE) return "phone is " + h.state();
                    String dim = h.pose().dimension();
                    return dim.equals("minecraft:the_nether") ? null : "pose in " + dim;
                }, "the phone is awake in the Nether (pose and modules there)", 5_000)
                .mutate(r -> {
                    FakePlayer t = st(r).other.entity();
                    t.setServerLevel(r.level());
                    standAt(st(r).other, r, new Vec3(3.5, 0, 0.5));
                    int slot = slotOf(t, st(r).phone);
                    ItemStack s = t.getInventory().removeItemNoUpdate(slot);
                    t.drop(s, false);
                }, "back home, the second player drops the phone")
                .await(r -> {
                    PhoneHost h = host(r);
                    return h.state() == PhoneState.OFF ? null : "phone is " + h.state();
                }, "dropped (and the other copy in an inventory): off", 5_000)
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ phone_wifi_walk

    static final String WALK_SSID = "ecm-walk", GW_IP = "192.168.78.1", PHONE_IP = "192.168.78.2";
    static final BlockPos WALK_AP = new BlockPos(4, 1, 2), WALK_GW = new BlockPos(4, 1, 5);
    static final List<BlockPos> WALK_CABLE = List.of(new BlockPos(4, 0, 5), new BlockPos(4, 0, 4), new BlockPos(4, 0, 3), new BlockPos(4, 0, 2));
    /** Open-air end of the walk (28 blocks from the AP), and behind the wall. */
    static final double FAR_X = 32.5, HIDDEN_X = 44.5;
    static final List<BlockPos> WALL = PlayerKit.box(36, 1, -4, 39, 6, 4);

    private static Scenario wifiWalk() {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -4, 46, 0, 6));
        floor.removeAll(WALK_CABLE);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(WALK_CABLE);
        foot.add(WALK_AP);
        foot.addAll(WALL);
        var b = Scenario.builder("phone_wifi_walk",
                        "the phone's Wi-Fi stays associated while its holder walks 28 blocks from an Access Point, and drops behind a stone wall")
                .asPlayer()
                .realTime()
                .host("gw", WALK_GW, GW_IP + "/24")
                .decor(PlayerKit.decor(foot, r -> {
                    PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState());
                    PlayerKit.fill(r, WALL, Blocks.STONE.defaultBlockState());
                }, r -> {
                    for (BlockPos p : WALK_CABLE) r.player().place(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE.get(), r.abs(p), Direction.UP);
                    PlayerKit.accessPoint(r, WALK_AP, Direction.WEST, WALK_SSID, null, 6, 0);
                }, PhoneScenarios::cleanup))
                .note("Setup (done for you): gw computer cabled to an open Access Point (" + WALK_SSID + ", channel 6, 0 dBm);"
                        + " a player holds a new phone (factory Wi-Fi Module) next to it; a stone wall 32 blocks east.")
                .send("gw", "ifconfig eth0 " + GW_IP + "/24")
                .expect("gw", "eth0: inet " + GW_IP.replace(".", "\\.") + "/24", "gw has " + GW_IP)
                .mutate(PhoneScenarios::givePhone, "the player takes a new phone and holds it (after opening gw's screen)")
                .await(PhoneScenarios::booted, "the phone boots", 40_000);
        clearScreen(b);
        b.mutate(r -> type(r, holder(r), "wifi connect " + WALK_SSID + "\n"), "on the phone: wifi connect " + WALK_SSID);
        b.await(r -> {
            ApPackets.ClientRow row = apRow(r);
            return row != null ? null : "the AP lists no client yet\n" + screen(r);
        }, "the phone joins " + WALK_SSID + ": the AP's screen lists it", 30_000);
        b.await(r -> {
            String last = screen(r).stripTrailing();
            return last.endsWith("/ >") ? null : "wifi still running";
        }, "wifi finishes (no DHCP on this network, so it says to set an address by hand)", 30_000);
        phoneRun(b, "ifconfig wlan0 " + PHONE_IP + "/24", "wlan0: inet " + PHONE_IP.replace(".", "\\."), null, "the phone has " + PHONE_IP, 10_000);
        b.await(pingUntil(1), "the phone pings gw through the AP", 20_000);
        b.note("Walk east, 0.2 blocks a tick, to 28 blocks from the AP (pinging all the way)");
        b.mutate(r -> st(r).since = System.currentTimeMillis(), "start walking");
        b.await(r -> {
            FakePlayer p = holder(r);
            double x = p.getX() - r.origin().getX();
            if (x < FAR_X) {
                standAt(r.player(), r, new Vec3(Math.min(FAR_X, x + 0.2), 0, 0.5));
                return "walking, x " + String.format(Locale.ROOT, "%.1f", x);
            }
            return null;
        }, "the holder walks to x 32.5 (28 blocks from the AP)", 20_000);
        b.await(r -> {
            ApPackets.ClientRow row = apRow(r);
            return row != null ? null : "the AP no longer lists the phone";
        }, "the AP still lists the phone, far away", 2_000);
        phoneRun(b, "ping " + GW_IP + " -n 3", "^3 packets sent, [1-3] received", "^3 packets sent, 0 received",
                "28 blocks out in the open: pings still answered", 20_000);
        b.mutate(PhoneScenarios::linkBudget, "/ecm radio link from the AP to both spots: open is over the -92 dBm sensitivity, behind the wall under it");
        b.note("Step behind the stone wall");
        b.mutate(r -> standAt(r.player(), r, new Vec3(HIDDEN_X, 0, 0.5)), "walk behind the wall");
        b.waitMs(2_000, "behind the wall");
        phoneRun(b, "ping " + GW_IP + " -n 3", "^3 packets sent, 0 received", "^3 packets sent, [1-3] received",
                "behind the wall the pings get no answer: past the link's range", 30_000);
        b.timeLimit(60_000);
        return b.build();
    }

    /** Ping gw from the phone until {@code n} replies arrive (an Await; retypes every 4 s). */
    private static java.util.function.Function<ScenarioRun, String> pingUntil(int n) {
        long[] sent = {0};
        return r -> {
            String s = screen(r);
            if (Pattern.compile("^\\d+ packets sent, [" + n + "-9] received", Pattern.MULTILINE).matcher(s).find()) return null;
            // Retype only at an idle prompt (the last line is the prompt): typing into a running ping is lost.
            String last = s.stripTrailing();
            last = last.substring(last.lastIndexOf('\n') + 1);
            if (last.strip().equals("/ >") && System.currentTimeMillis() - sent[0] > 3_000) {
                sent[0] = System.currentTimeMillis();
                type(r, holder(r), "ping " + GW_IP + " -n 1\n");
            }
            return "phone screen:\n" + s;
        };
    }

    private static ApPackets.ClientRow apRow(ScenarioRun r) {
        ApPackets.ApView v = r.player().accessPointScreen(r.abs(WALK_AP));
        if (v == null || v.clients().isEmpty()) return null;
        return v.clients().get(0);
    }

    static final Pattern TOTAL = Pattern.compile("total (-?\\d+\\.\\d) dB");

    /** The player's /ecm radio link from the AP to the open spot and behind the wall (2437 MHz). */
    private static void linkBudget(ScenarioRun r) {
        BlockPos ap = r.abs(WALK_AP);
        double[] loss = new double[2];
        double[] xs = {FAR_X, HIDDEN_X};
        for (int i = 0; i < 2; i++) {
            Vec3 at = Vec3.atLowerCornerOf(r.origin()).add(xs[i], 1.32, 0.5);
            String cmd = String.format(Locale.ROOT, "/ecm radio link %.1f %.1f %.1f %.1f %.1f %.1f 2437",
                    ap.getX() + 0.5, ap.getY() + 0.5, ap.getZ() + 0.5, at.x, at.y, at.z);
            String said = String.join("\n", r.player().command(cmd));
            Matcher m = TOTAL.matcher(said);
            if (!m.find()) {
                r.fail("/ecm radio link said: " + said);
                return;
            }
            loss[i] = Math.abs(Double.parseDouble(m.group(1)));
        }
        // AP 0 dBm, module sensitivity -92 dBm (WifiModule), antenna gains ignored (about 0 dBi each).
        double open = 0 - loss[0], hidden = 0 - loss[1];
        r.say(String.format(Locale.ROOT, "§f  link budget: open %.1f dBm, behind the wall %.1f dBm (sensitivity -92)", open, hidden));
        if (open < -92 + 3) r.fail(String.format(Locale.ROOT, "the open spot should be in range, budget %.1f dBm", open));
        if (hidden > -92) r.fail(String.format(Locale.ROOT, "behind the wall should be out of range, budget %.1f dBm", hidden));
    }

    // Last: the scenarios use the constants above.
    static {
        for (Scenario s : List.of(basics(), battery(), visibility(), duplicates(), wifiWalk())) ALL.put(s.name, s);
    }
}
//?}
