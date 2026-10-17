package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.card.CardContent;
import com.example.evanscomputermod.card.CardReaderBlock;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.phys.Vec3;

import java.util.ArrayList;
import java.util.Collections;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;
import java.util.regex.Matcher;
import java.util.regex.Pattern;

/**
 * Telephony (docs/TELEPHONY.md) on a player-built carrier: a core computer running cored (with
 * a number block and the IMS bearer set up with cored's own commands), imsd, smscd, mmscd and
 * mrfd, a Card Reader/Writer for its SIMs and a cell site (celld-site, Advanced SDR, coax, LTE
 * Sector Antenna). Each phone gets a SIM from {@code cored sim new} (which gives it the block's
 * next number), an LTE Modem, attaches through celld and registers with imsd (telephonyd).
 *
 * <p>Two or three people: the scenario player (Ann) and second scenario players (Bea, Cal) with
 * their own ePhones [exception: second scenario players, docs/SCENARIO_EXCEPTIONS.md]. Ann writes
 * every SIM at the core and hands Bea's and Cal's over.
 *
 * <ul>
 *   <li>{@code tel_register}: one phone registers (VoLTE in the status bar, {@code tel status} shows
 *       its number, {@code cell status} the IMS bearer). Control: {@code cored ims off} and the SIM
 *       out and in again: attached, but no IMS bearer and no VoLTE.</li>
 *   <li>{@code tel_call}: Ann dials Bea on the Phone app's keypad; Bea's phone shows the incoming
 *       call, she taps Answer; both are in the call; {@code tel calls} on Ann's phone shows the voice
 *       packet counters moving both ways; Ann hangs up and Bea sees Call ended. Control: Ann calls
 *       again, Bea taps Decline, Ann's call log says declined.</li>
 *   <li>{@code tel_call_asleep}: Bea's phone in her pocket (asleep); Ann calls; the cell pages it,
 *       it wakes and rings; Bea takes it out and answers.</li>
 *   <li>{@code tel_sms}: Ann texts Bea in Messages (New, the number, Add, the text, Send); Bea gets a
 *       notification and the thread; Ann's bubble says Delivered. Store and forward: Bea's SIM out,
 *       Ann texts again (Sent, not Delivered), the SIM back in, the text arrives, Delivered.</li>
 *   <li>{@code tel_mms_group}: Ann sends a picture to Bea and Cal's number as a group ({@code sms mms});
 *       Bea gets it in a group thread with the picture.</li>
 *   <li>{@code tel_scripts}: a Lua one-liner with {@code phone.on_sms} waits on Bea's phone; a Python
 *       one-liner with {@code phone.sms_send} on Ann's sends the text; Lua prints it.</li>
 *   <li>{@code tel_ivr}: Ann calls 611 from the keypad, presses 2 on the in-call keypad, and gets a
 *       text with her number.</li>
 * </ul>
 */
public final class TelephonyScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();
    static final String PROMPT = PlayerKit.PROMPT;
    static final String CARRIER = "cell-red", POOL = "100.65.2.0/24", BLOCK = "214-555", IMS_POOL = "10.250.0.0/16", DOMAIN = "red.ecm";
    static final String CORE_IP = "10.0.40.1", SITE_IP = "10.0.40.2";
    static final BlockPos CORE = new BlockPos(0, 1, -6), READER = CORE.east(), SITE = FramedScenarios.A;
    static final String DIALER = "dev.ecm.dialer", MESSAGES = "dev.ecm.messages";
    static final String ANN = "Ann", BEA = "Bea", CAL = "Cal";
    /** Where each person stands: 17-20 blocks east of the site's antenna. */
    static final Map<String, Vec3> SPOT = Map.of(ANN, new Vec3(20.5, 0, 0.5), BEA, new Vec3(20.5, 0, 3.5), CAL, new Vec3(20.5, 0, -2.5));

    private static final Map<ScenarioRun, Map<String, Handset>> RUNS = Collections.synchronizedMap(new WeakHashMap<>());

    private TelephonyScenarios() {}

    static Handset hand(ScenarioRun r, String who) {
        return RUNS.computeIfAbsent(r, k -> new LinkedHashMap<>())
                .computeIfAbsent(who, w -> new Handset(w, w.equals(ANN) ? null : new ScenarioPlayer(r.level(), w)));
    }

    /** A number's local form (dialled without +1 and the area code): +12145550101 -> 5550101. */
    static String local(String e164) {
        return e164.length() >= 7 ? e164.substring(e164.length() - 7) : e164;
    }

    static String digits(String s) {
        return s.replaceAll("\\D", "");
    }

    static void cleanup(ScenarioRun r) {
        Map<String, Handset> hs = RUNS.remove(r);
        if (hs != null) for (Handset h : hs.values()) h.forget(r);
        PhoneScenarios.cleanup(r);
    }

    // ------------------------------------------------------------ the network

    static List<BlockPos> floor() {
        List<BlockPos> f = new ArrayList<>(PlayerKit.box(-2, 0, -8, 24, 0, 6));
        f.removeAll(backhaul().blocks());
        return f;
    }

    static Scenario.Path backhaul() {
        return Scenario.Path.from(new BlockPos(0, 0, -6)).go(Direction.SOUTH, 6);
    }

    static Scenario.Builder start(String name, String what) {
        List<BlockPos> floor = floor();
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(backhaul().blocks());
        foot.addAll(FramedScenarios.station(SITE));
        foot.add(READER);
        return Scenario.builder(name, what)
                .asPlayer()
                .realTime()
                .host("core", CORE, "-")
                .host("site", SITE, "-")
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, backhaul())
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    FramedScenarios.build(r, SITE);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                }, TelephonyScenarios::cleanup))
                .note("Setup (done for you): the core computer cabled (bottom) to the cell site's computer; a Card Reader/Writer"
                        + " against the core's east side; beside the site an Advanced SDR, coax and an LTE Sector Antenna pointing east.");
    }

    /**
     * Everyone takes a new ePhone; the core (cored with a number block and IMS, imsd, smscd, mmscd, mrfd) and the site
     * come up; Ann writes a SIM per person ({@code cored sim new}: an IMSI and the block's next number) and hands
     * the others theirs and an LTE Modem; each fits modem and SIM in their phone; every phone attaches and registers
     * (VoLTE). {@code simOnly} people get a SIM (a number) but no phone.
     */
    static void network(Scenario.Builder b, List<String> people, List<String> simOnly) {
        b.mutate(r -> {
            hand(r, ANN).givePhone(r, SPOT.get(ANN));
            hand(r, ANN).phoneToOffHand(r);
        }, "Ann takes a new ePhone (it boots while she builds) and puts it in her off hand");
        for (String p : people) {
            if (p.equals(ANN)) continue;
            b.mutate(r -> hand(r, p).givePhone(r, SPOT.get(p)), p + " takes a new ePhone and holds it, " + (int) SPOT.get(p).x
                    + " blocks east [exception: a second scenario player, docs/SCENARIO_EXCEPTIONS.md]");
        }
        b.send("site", "ifconfig eth0 " + SITE_IP + "/24");
        b.send("core", "ifconfig eth0 " + CORE_IP + "/24");
        b.expect("core", "eth0: inet 10\\.0\\.40\\.1/24", "the core has " + CORE_IP);
        b.expect("site", "eth0: inet 10\\.0\\.40\\.2/24", "the site has " + SITE_IP);
        b.note("core: the subscriber database, a number block, the IMS bearer, site-a's credential, the core");
        b.send("core", "cored init " + CARRIER + " " + POOL);
        b.expectOrFail("core", "^cored: carrier cell-red pool 100\\.65\\.2\\.0/24", "cored writes /etc/cored.db", "^cored: (?!carrier)");
        b.send("core", "cored block add " + BLOCK);
        b.expectOrFail("core", "^block 214-555 added", "the number block 214-555", "^cored: ");
        b.send("core", "cored ims pool=" + IMS_POOL + " domain=" + DOMAIN);
        b.expectOrFail("core", "^ims pool 10\\.250\\.0\\.0/16 pcscf \\S+ domain red\\.ecm", "the IMS bearer (VoLTE, texts)", "^cored: ");
        b.send("core", "cored site-add site-a " + CellularScenarios.secret("site-a"));
        b.expect("core", "site site-a added", "site-a's credential");
        b.send("core", "cored start &");
        b.expect("core", "cored: carrier cell-red pool 100\\.65\\.2\\.0/24, cell0 ", "cored runs");
        b.note("core: the IMS servers beside cored (calls, texts, picture messages, voicemail and 611)");
        for (String[] d : new String[][]{{"imsd", "imsd: red\\.ecm on "}, {"smscd", "smscd: SMSC of red\\.ecm on "},
                {"mmscd", "mmscd: MMSC of red\\.ecm at "}, {"mrfd", "mrfd: media server of red\\.ecm on "}}) {
            b.send("core", d[0] + " start &");
            b.expectOrFail("core", d[1], d[0] + " runs", "^" + d[0] + ": (the HSS|can't|bind|no )");
        }
        CellularScenarios.siteUp(b, "site", "site-a", CARRIER, CORE_IP, 1, PhyProfile.LTE, 1, "");
        List<String> all = new ArrayList<>(people);
        all.addAll(simOnly);
        for (String p : all) writeSim(b, p, people.contains(p));
        for (String p : people) b.await(r -> hand(r, p).guiBooted(r), p + "'s phone has booted into its shell", 40_000);
        b.mutate(r -> {
            Handset a = hand(r, ANN);
            a.standAt(r, SPOT.get(ANN));
            a.phoneToHand(r);
        }, "Ann walks 20 blocks east of the antenna, the phone back in her main hand");
        for (String p : people) {
            b.note(p + ": sneak-right-click the phone (its slot screen), shift-click the LTE Modem and the SIM into it");
            b.mutate(r -> hand(r, p).fitModemAndSim(r), p + "'s slot screen: LTE Modem and SIM in");
        }
        for (String p : people) {
            b.await(r -> {
                PhoneUi u = hand(r, p).ui(r);
                return u != null && u.celldPid() > 0 && u.telephonydPid() > 0 ? null : p + ": events " + (u == null ? "-" : u.events());
            }, p + "'s phone starts celld and telephonyd by itself", 15_000);
            b.await(r -> hand(r, p).statusHas(r, "\ncarrier " + CARRIER + "\n"), p + "'s status bar shows " + CARRIER + " (attached)", 25_000);
        }
        for (String p : people) b.await(r -> volte(r, p), p + "'s phone registers: VoLTE and the number in the status bar", 25_000);
    }

    /** Null once the status bar has VoLTE and the person's number. */
    static String volte(ScenarioRun r, String p) {
        Handset h = hand(r, p);
        String why = h.statusHas(r, "\nvolte 1\nnumber " + h.number + "\n");
        return why == null ? null : why + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 1200) + "\ncelld.log:\n" + h.log(r, "var/log/celld.log", 600);
    }

    static final Pattern WROTE = Pattern.compile("^cored: wrote SIM IMSI (\\d{15}) \\((\\w+)\\), number (\\+\\d+)", Pattern.MULTILINE);

    /** A blank SIM into the core's reader, {@code cored sim new --label}, taken out; someone else's handed over with an LTE Modem. */
    static void writeSim(Scenario.Builder b, String p, boolean phone) {
        String label = p.toLowerCase(java.util.Locale.ROOT);
        b.note("core: a SIM for " + p + ". A blank card clicked into the Card Reader/Writer, cored sim new, taken out by hand");
        b.mutate(r -> CellularScenarios.insertBlank(r, READER), "right-click the Card Reader/Writer with a blank SIM card");
        b.send("core", "cored sim new --label " + label);
        b.expectOrFail("core", "^cored: wrote SIM IMSI \\d{15} \\(" + label + "\\), number \\+1214555\\d{4}",
                "cored writes the SIM and gives it the block's next number", "^cored: (?!wrote|the card)");
        b.mutate(r -> {
            Matcher m = WROTE.matcher(r.screen("core"));
            String imsi = null, number = null;
            while (m.find()) if (m.group(2).equals(label)) {
                imsi = m.group(1);
                number = m.group(3);
            }
            if (imsi == null) {
                r.fail("no SIM for " + label + " on the core's screen");
                return;
            }
            Handset h = hand(r, p);
            h.imsi = imsi;
            h.number = number;
            Inventory ann = r.player().entity().getInventory();
            ann.selected = 8;
            ann.setItem(8, ItemStack.EMPTY);
            r.player().useEmpty(r.abs(READER), Direction.NORTH, false);
            int s = Handset.slotOfSim(ann, imsi);
            if (s < 0) {
                r.fail("taking the card out gave Ann no SIM " + imsi + ": " + r.player().heard());
                return;
            }
            if (!phone) {
                r.say("§7  " + p + "'s SIM (" + number + ") stays in Ann's inventory: a number with no phone");
                return;
            }
            Inventory to = h.inv(r);
            ItemStack sim = ann.getItem(s);
            ann.setItem(s, ItemStack.EMPTY);
            to.setItem(20, sim);
            to.setItem(21, new ItemStack(CellularContent.modem(PhyProfile.LTE)));
            r.say("§7  " + p + " has SIM " + imsi + " (number " + number + ") and an LTE Modem");
        }, "take the SIM out of the reader (empty hand)" + (p.equals(ANN) || !phone ? "" : " and hand it to " + p + " with an LTE Modem"));
    }

    // ------------------------------------------------------------ helpers for the steps

    /** Tap the keypad digits of {@code number} and then Call, in the Phone app's Keypad tab. */
    static void dialOnKeypad(Scenario.Builder b, String who, java.util.function.Function<ScenarioRun, String> number, String shown) {
        b.await(r -> hand(r, who).tap(r, "Phone", "Keypad"), who + ": tap the Keypad tab", 10_000);
        b.await(r -> hand(r, who).shows(r, "Phone", "Delete"), who + ": the keypad is shown", 5_000);
        b.mutate(r -> {
            Handset h = hand(r, who);
            for (char c : number.apply(r).toCharArray()) {
                String why = h.tap(r, "Phone", String.valueOf(c));
                if (why != null) {
                    r.fail(why);
                    return;
                }
            }
        }, who + ": tap " + shown + " on the keypad");
        b.await(r -> {
            String typed = digits(String.join(" ", hand(r, who).texts(r, "Phone")));
            return typed.contains(number.apply(r)) ? null : who + "'s keypad shows " + hand(r, who).texts(r, "Phone");
        }, who + ": the number is typed", 5_000);
        b.await(r -> hand(r, who).tap(r, "Phone", "Call"), who + ": tap Call", 5_000);
    }

    /** Null while {@code who}'s Phone app is in a call (its in-call buttons). */
    static String inCall(ScenarioRun r, String who) {
        Handset h = hand(r, who);
        List<String> t = h.texts(r, "Phone");
        return t.contains("End") && t.contains("Mute") ? null : who + "'s Phone shows " + t;
    }

    static final Pattern VOICE = Pattern.compile("voice in (\\d+) out (\\d+) packets");

    /** {@code tel calls} on {@code who}'s Terminal: the call active, voice packets both ways, and more coming in. */
    static void voiceFlows(Scenario.Builder b, String who, String what) {
        b.mutate(r -> hand(r, who).mark = 0, "-");
        for (int round = 0; round < 2; round++) {
            final int n = round;
            clear(b, who);
            b.mutate(r -> hand(r, who).type(r, "tel calls\n"), who + "'s Terminal: tel calls");
            b.await(r -> {
                Handset x = hand(r, who);
                String joined = x.screen(r).replace("\n", " ");
                Matcher m = VOICE.matcher(joined);
                if (!joined.contains("active") || !m.find()) return who + "'s screen:\n" + x.screen(r);
                long in = Long.parseLong(m.group(1)), out = Long.parseLong(m.group(2));
                if (n == 0) {
                    if (in == 0 || out == 0) return who + ": no voice yet:\n" + x.screen(r);
                    x.mark = in;
                    x.memo = in + "/" + out;
                    return null;
                }
                return in > x.mark ? null : who + ": the counter didn't move (" + x.memo + " then " + in + "/" + out + ")";
            }, n == 0 ? what : who + ": tel calls again: more voice packets came in", 10_000);
        }
    }

    static void openApp(Scenario.Builder b, String who, String label, String appId) {
        b.mutate(r -> {
            PhoneUi u = hand(r, who).ui(r);
            if (u != null && u.foreground() != null) hand(r, who).altKey(r, PhoneGuiScenarios.HID_H);
        }, who + ": Alt+H (home) if an app is open");
        b.await(r -> hand(r, who).tapLayer(r, "home", label, false), who + ": tap " + label + " on the home grid", 20_000);
        b.await(r -> hand(r, who).onScreen(r, appId) != null ? null : who + "'s windows " + hand(r, who).comp(r).windows(),
                who + ": " + label + " opens full screen", 30_000);
    }

    static void openTerminal(Scenario.Builder b, String who) {
        openApp(b, who, "Terminal", PhoneScenarios.TERMINAL);
        b.await(r -> hand(r, who).screen(r).contains("/ >") ? null : who + "'s Terminal:\n" + hand(r, who).screen(r),
                who + ": the Terminal's shell prompt", 20_000);
    }

    static void clear(Scenario.Builder b, String who) {
        b.mutate(r -> hand(r, who).type(r, "clear\n"), who + "'s Terminal: clear");
        b.await(r -> hand(r, who).screen(r).strip().equals("/ >") ? null : who + "'s screen:\n" + hand(r, who).screen(r),
                who + "'s screen is clear", 10_000);
    }

    /**
     * What the Terminal printed after the typed {@code line} (its echo may wrap: the 30 columns): the rows
     * after the row where the echo ends, or null while the echo isn't on the screen yet.
     */
    static String after(String screen, String line) {
        String[] rows = screen.split("\n", -1);
        StringBuilder joined = new StringBuilder();
        int[] ends = new int[rows.length];
        for (int i = 0; i < rows.length; i++) {
            joined.append(rows[i]);
            ends[i] = joined.length();
        }
        int at = joined.lastIndexOf(line);
        if (at < 0) return null;
        int end = at + line.length(), row = 0;
        while (row < rows.length - 1 && ends[row] < end) row++;
        return String.join("\n", java.util.Arrays.copyOfRange(rows, row + 1, rows.length));
    }

    /**
     * A shell line on {@code who}'s Terminal (after clear); wait for {@code re} in what it printed after the
     * line (rows also joined), so a pattern like the prompt can't match before the command has run.
     */
    static void run(Scenario.Builder b, String who, java.util.function.Function<ScenarioRun, String> line, String shown, String re,
                    String fail, String what, int timeoutMs) {
        Pattern want = Pattern.compile(re, Pattern.MULTILINE), bad = fail == null ? null : Pattern.compile(fail, Pattern.MULTILINE);
        clear(b, who);
        b.mutate(r -> {
            Handset h = hand(r, who);
            h.typed = line.apply(r);
            if (!h.type(r, h.typed + "\n")) r.fail(who + "'s phone refused the typing");
        }, who + "'s Terminal: " + shown);
        b.await(r -> {
            Handset h = hand(r, who);
            String screen = h.screen(r), s = after(screen, h.typed);
            if (s == null) return who + "'s screen doesn't show the line yet:\n" + screen;
            String joined = s.replace("\n", "");
            if (bad != null && (bad.matcher(s).find() || bad.matcher(joined).find())) {
                r.fail(what + ": " + who + "'s phone printed\n" + screen);
                return null;
            }
            return want.matcher(s).find() || want.matcher(joined).find() ? null : who + "'s screen:\n" + screen;
        }, what, timeoutMs);
    }

    static void run(Scenario.Builder b, String who, String line, String re, String fail, String what, int timeoutMs) {
        run(b, who, r -> line, line, re, fail, what, timeoutMs);
    }

    /** Null once {@code who} has a notification of {@code app} whose title or body contains {@code text}. */
    static String notified(ScenarioRun r, String who, String app, String text) {
        List<Compositor.PostedNotification> n = hand(r, who).notifications(r);
        return n.stream().anyMatch(x -> x.appId().equals(app) && (x.body().contains(text) || x.title().contains(text))) ? null
                : who + "'s notifications " + n;
    }

    // ------------------------------------------------------------ tel_register

    static Scenario register() {
        var b = start("tel_register",
                "a carrier core with a number block and IMS (cored, imsd, smscd, mmscd, mrfd) and a cell site: cored sim new gives a SIM"
                        + " a number; the phone attaches and registers (VoLTE in the status bar, tel status shows the number); control:"
                        + " cored ims off, the SIM out and in: attached, no VoLTE");
        network(b, List.of(ANN), List.of());
        openTerminal(b, ANN);
        run(b, ANN, "tel status", "telephonyd: registered \\(VoLTE\\)", "telephonyd is not running", "tel status: registered (VoLTE)", 15_000);
        b.await(r -> {
            Handset a = hand(r, ANN);
            String s = a.screen(r);
            return digits(s).contains(digits(a.number).substring(1)) && s.contains("red.ecm") ? null : "Ann's screen:\n" + s;
        }, "tel status shows Ann's number and the network red.ecm", 5_000);
        run(b, ANN, "cell status", "IMS\\s+10\\.250\\.\\d+\\.\\d+", "celld is not running", "cell status: the IMS bearer from 10.250.0.0/16", 15_000);
        b.note("Control: the carrier turns IMS off (cored ims off); Ann's phone attaches again (SIM out and in): no IMS bearer, no VoLTE");
        b.mutate(r -> hand(r, ANN).phoneToOffHand(r), "Ann: the phone into her off hand, to type at the core");
        b.send("core", "cored ims off");
        b.expect("core", "^ims off \\(phones get no IMS bearer\\)", "cored: IMS off");
        b.mutate(r -> {
            Handset a = hand(r, ANN);
            a.standAt(r, SPOT.get(ANN));
            a.phoneToHand(r);
            a.simOut(r);
        }, "Ann walks back; slot screen: the SIM out");
        b.await(r -> hand(r, ANN).statusHas(r, "\nvolte 0\n"), "no SIM: no VoLTE", 15_000);
        b.mutate(r -> hand(r, ANN).simIn(r), "slot screen: the SIM back in");
        b.await(r -> hand(r, ANN).statusHas(r, "\ncarrier " + CARRIER + "\n"), "attached again", 30_000);
        b.mutate(r -> hand(r, ANN).mark = System.currentTimeMillis(), "-");
        b.await(r -> {
            Handset a = hand(r, ANN);
            if (a.statusHas(r, "\nvolte 1\n") == null) {
                r.fail("VoLTE with IMS off:\n" + a.status(r));
                return null;
            }
            return System.currentTimeMillis() - a.mark > 6_000 ? null : "watching";
        }, "for 6 s attached and still no VoLTE (no IMS bearer)", 10_000);
        run(b, ANN, "cell status", "state\\s+connected", "IMS\\s+10\\.250", "cell status: connected, no IMS line", 10_000);
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_call

    static Scenario call() {
        var b = start("tel_call",
                "two phones on one carrier: Ann dials Bea on the Phone app's keypad, Bea's phone shows the incoming call and she answers;"
                        + " voice flows both ways (tel calls' packet counters); Ann hangs up; control: Bea declines, Ann's log says declined");
        network(b, List.of(ANN, BEA), List.of());
        b.note("Ann: Phone, Keypad, Bea's number (local: without +1 and the area code), Call");
        openApp(b, ANN, "Phone", DIALER);
        dialOnKeypad(b, ANN, r -> local(hand(r, BEA).number), "Bea's number");
        b.await(r -> {
            Handset h = hand(r, BEA);
            if (h.onScreen(r, DIALER) == null) return "Bea's Phone isn't on screen: " + (h.comp(r) == null ? "-" : h.comp(r).windows());
            String w = h.shows(r, "Phone", "Incoming call");
            if (w != null) return w;
            PhoneUi u = h.ui(r);
            return u != null && u.ringingWho() != null ? null : "Bea's phone doesn't ring";
        }, "Bea's phone rings: the incoming-call screen fills it", 20_000);
        b.await(r -> hand(r, BEA).shows(r, "Phone", digits(hand(r, ANN).number).substring(4, 7)), "it shows Ann's number", 3_000);
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Answer"), "Bea taps Answer", 5_000);
        b.await(r -> inCall(r, BEA), "Bea is in the call (in-call screen)", 10_000);
        b.await(r -> inCall(r, ANN), "Ann is in the call (in-call screen)", 10_000);
        b.note("Voice: on Ann's phone, home and Terminal (the call keeps going): tel calls twice, the voice counters move both ways");
        openTerminal(b, ANN);
        voiceFlows(b, ANN, "tel calls: the call to Bea is active, voice packets both ways");
        b.note("Ann hangs up (tel hangup): Bea's phone shows Call ended");
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "tel hangup", 10_000);
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Call ended"), "Bea's Phone: Call ended", 10_000);
        run(b, ANN, "tel calls", "tel: no calls", null, "tel calls: no calls", 10_000);
        b.note("Control: Ann calls again (tel dial), Bea taps Decline; Ann's call ends and her log says declined");
        run(b, ANN, r -> "tel dial " + local(hand(r, BEA).number), "tel dial <Bea>", "tel: calling", "^tel: (?!calling)", "tel dial: calling Bea", 10_000);
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Incoming call"), "Bea's phone rings again", 20_000);
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Decline"), "Bea taps Decline", 5_000);
        b.await(r -> {
            Handset h = hand(r, BEA);
            PhoneUi u = h.ui(r);
            if (u != null && u.ringingWho() == null && h.shows(r, "Phone", "Incoming call") != null) return null;
            return "Bea's phone still rings: " + h.texts(r, "Phone") + " windows " + h.comp(r).windows() + "\ntelephonyd.log:\n"
                    + h.log(r, "var/log/telephonyd.log", 600);
        }, "Bea's phone stops ringing (one Phone app: the running one was raised, not started again)", 5_000);
        b.await(r -> hand(r, BEA).comp(r).windows().stream().filter(w -> w.appId().equals(DIALER)).count() == 1 ? null
                : "Bea's windows " + hand(r, BEA).comp(r).windows(), "Bea has one Phone window", 1_000);
        b.note("A declined call sounds busy to the caller (the network doesn't tell her it was declined); the core's CDR says declined");
        run(b, ANN, "tel log 1", "out .*busy", null, "Ann's tel log: the call out, busy", 15_000);
        b.send("core", "cat /var/log/cdr/imsd.cdr");
        b.expect("core", "result=declined", "the core's call record: declined");
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_call_asleep

    static Scenario callAsleep() {
        var b = start("tel_call_asleep",
                "Bea's phone in her pocket (asleep); Ann calls; the cell pages it, it wakes and rings; Bea takes it out and answers");
        network(b, List.of(ANN, BEA), List.of());
        b.note("Bea puts her phone in her pocket (main inventory): asleep, its computer frozen");
        b.mutate(r -> {
            Handset h = hand(r, BEA);
            h.movePhone(r, 9);
            h.inv(r).selected = 0;
        }, "Bea: the phone into her pocket");
        b.await(r -> {
            PhoneHost h = hand(r, BEA).host(r);
            return h != null && h.state() == PhoneState.ASLEEP ? null : "Bea's phone is " + (h == null ? "-" : h.state());
        }, "Bea's phone is asleep", 5_000);
        b.waitMs(3_000, "let it settle into idle");
        openTerminal(b, ANN);
        run(b, ANN, r -> "tel dial " + local(hand(r, BEA).number), "tel dial <Bea>", "tel: calling", "^tel: (?!calling)", "Ann: tel dial Bea", 10_000);
        b.await(r -> {
            Handset h = hand(r, BEA);
            PhoneHost ph = h.host(r);
            PhoneUi u = h.ui(r);
            if (ph == null || ph.state() != PhoneState.ASLEEP) return "Bea's phone is " + (ph == null ? "-" : ph.state());
            return u != null && u.ringingWho() != null ? null : "Bea's phone doesn't ring\ncelld.log:\n" + h.log(r, "var/log/celld.log", 800)
                    + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 800);
        }, "paged: Bea's phone wakes in her pocket and rings", 25_000);
        b.mutate(r -> hand(r, BEA).phoneToHand(r), "Bea takes the phone out (main hand)");
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Incoming call"), "Bea's phone shows the incoming call", 10_000);
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Answer"), "Bea taps Answer", 5_000);
        b.await(r -> inCall(r, BEA), "Bea is in the call", 10_000);
        run(b, ANN, "tel calls", "active", null, "Ann's tel calls: active", 10_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_sms

    static final String TEXT1 = "lunch at noon?", TEXT2 = "you there now?";

    /** Ann's Messages: the text right after {@code text} in its window (the bubble's status line, "now · Delivered"), or "". */
    static String statusAfter(ScenarioRun r, String text) {
        List<String> t = hand(r, ANN).texts(r, "Messages");
        int i = t.lastIndexOf(text);
        return i >= 0 && i + 1 < t.size() ? t.get(i + 1) : "";
    }

    static Scenario sms() {
        var b = start("tel_sms",
                "Ann texts Bea in Messages; Bea gets a notification and the thread; Ann sees Delivered. Store and forward: Bea's SIM"
                        + " out, Ann texts (Sent), the SIM back in: the text arrives, Delivered");
        network(b, List.of(ANN, BEA), List.of());
        b.note("Ann: Messages, New, To: Bea's number, Add, the text, Send");
        openApp(b, ANN, "Messages", MESSAGES);
        b.await(r -> hand(r, ANN).tap(r, "Messages", "New"), "Ann taps New", 10_000);
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Name or number"), "Ann taps To", 5_000);
        b.mutate(r -> hand(r, ANN).type(r, local(hand(r, BEA).number)), "Ann types Bea's number");
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Add"), "Ann taps Add", 5_000);
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Message"), "Ann taps the message box", 5_000);
        b.mutate(r -> hand(r, ANN).type(r, TEXT1), "Ann types \"" + TEXT1 + "\"");
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Send"), "Ann taps Send", 5_000);
        b.await(r -> notified(r, BEA, MESSAGES, TEXT1), "Bea gets a notification with the text", 20_000);
        b.await(r -> statusAfter(r, TEXT1).endsWith("Delivered") ? null : "Ann's Messages: " + hand(r, ANN).texts(r, "Messages"),
                "Ann's bubble says Delivered (the delivery report)", 20_000);
        b.note("Bea opens Messages: the thread with Ann");
        openApp(b, BEA, "Messages", MESSAGES);
        b.await(r -> hand(r, BEA).shows(r, "Messages", TEXT1), "Bea's conversation list shows the text", 10_000);
        b.await(r -> hand(r, BEA).tap(r, "Messages", TEXT1), "Bea taps the conversation", 5_000);
        b.await(r -> hand(r, BEA).shows(r, "Messages", "Text message"), "Bea's thread opens (reply box)", 10_000);
        b.note("Store and forward: Bea's SIM out (no service); Ann texts again: Sent, not Delivered");
        b.mutate(r -> hand(r, BEA).simOut(r), "Bea: slot screen, the SIM out");
        b.await(r -> hand(r, BEA).ui(r).telephonydPid() < 0 ? null : "telephonyd still runs", "Bea's telephonyd stops (no SIM)", 10_000);
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Text message"), "Ann taps the reply box in the thread", 10_000);
        b.mutate(r -> hand(r, ANN).type(r, TEXT2), "Ann types \"" + TEXT2 + "\"");
        b.await(r -> hand(r, ANN).tap(r, "Messages", "Send"), "Ann taps Send", 5_000);
        b.await(r -> statusAfter(r, TEXT2).endsWith("Sent") ? null : "Ann's Messages: " + hand(r, ANN).texts(r, "Messages"),
                "Ann's second bubble: Sent (the SMSC has it)", 15_000);
        b.mutate(r -> hand(r, ANN).mark = System.currentTimeMillis(), "-");
        b.await(r -> {
            if (statusAfter(r, TEXT2).endsWith("Delivered")) r.fail("Delivered while Bea's phone has no SIM");
            return System.currentTimeMillis() - hand(r, ANN).mark > 3_000 ? null : "watching";
        }, "for 3 s it stays Sent", 5_000);
        b.mutate(r -> hand(r, BEA).simIn(r), "Bea: the SIM back in");
        b.await(r -> volte(r, BEA), "Bea's phone registers again", 40_000);
        b.await(r -> notified(r, BEA, MESSAGES, TEXT2), "the stored text reaches Bea", 30_000);
        b.await(r -> statusAfter(r, TEXT2).endsWith("Delivered") ? null : "Ann's Messages: " + hand(r, ANN).texts(r, "Messages"),
                "Ann's second bubble: Delivered", 20_000);
        b.timeLimit(150_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_mms_group

    static final String PIC_TEXT = "the new sign";

    static Scenario mmsGroup() {
        var b = start("tel_mms_group",
                "Ann sends a picture to Bea and Cal as a group (sms mms); Bea gets it in a group thread with the picture. Cal is a"
                        + " number with no phone here (his SIM is written; the MMSC keeps his copy)");
        network(b, List.of(ANN, BEA), List.of(CAL));
        b.note("Ann's Terminal: a picture (a copy of the Notes app's icon) to Bea and Cal");
        openTerminal(b, ANN);
        run(b, ANN, "cp /apps/dev.ecm.notes/icon.png /home/sign.png", PROMPT, "^cp: ", "a picture at /home/sign.png", 10_000);
        run(b, ANN, r -> "sms mms " + local(hand(r, BEA).number) + "," + local(hand(r, CAL).number) + " --file /home/sign.png " + PIC_TEXT,
                "sms mms <Bea>,<Cal> --file /home/sign.png " + PIC_TEXT, "sms: MMS sent to", "^sms: (?!MMS sent)", "sms: the MMS is sent", 30_000);
        b.await(r -> notified(r, BEA, MESSAGES, PIC_TEXT), "Bea gets the picture message (notification)", 30_000);
        openApp(b, BEA, "Messages", MESSAGES);
        b.await(r -> hand(r, BEA).tap(r, "Messages", PIC_TEXT), "Bea taps the conversation", 10_000);
        b.await(r -> hand(r, BEA).shows(r, "Messages", "View"), "the thread shows the picture (View)", 10_000);
        b.await(r -> {
            Handset bea = hand(r, BEA);
            String log = bea.log(r, "var/telephony/messages.log", 4000);
            String a = hand(r, ANN).number, c = hand(r, CAL).number;
            return log.contains(a.compareTo(c) < 0 ? a + "," + c : c + "," + a) ? null : "Bea's messages.log:\n" + log;
        }, "it is one group thread: Ann and Cal (the thread's key on Bea's phone)", 5_000);
        b.await(r -> {
            try (var s = java.nio.file.Files.walk(hand(r, BEA).computer(r).storagePath().resolve("home/Messages"))) {
                return s.anyMatch(p -> p.toString().endsWith(".png")) ? null : "no picture under Bea's /home/Messages";
            } catch (Exception e) {
                return "Bea's /home/Messages: " + e;
            }
        }, "the picture is saved on Bea's phone (/home/Messages)", 5_000);
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_scripts

    static final String SCRIPT_TEXT = "hi from python";

    static Scenario scripts() {
        var b = start("tel_scripts",
                "scripts: a Lua one-liner with phone.on_sms waits on Bea's phone; a Python one-liner with phone.sms_send on Ann's sends"
                        + " a text; Lua prints it");
        network(b, List.of(ANN, BEA), List.of());
        openTerminal(b, BEA);
        clear(b, BEA);
        b.mutate(r -> hand(r, BEA).type(r, "lua\n"), "Bea's Terminal: lua");
        b.await(r -> {
            String s = after(hand(r, BEA).screen(r), "/ > lua");
            return s != null && s.lines().anyMatch(l -> l.strip().equals(">")) ? null : "Bea's screen:\n" + hand(r, BEA).screen(r);
        }, "the Lua prompt", 15_000);
        String lua = "phone.on_sms(function(f, t) print('lua got ' .. t .. ' from ' .. f) end) phone.run()";
        b.mutate(r -> hand(r, BEA).type(r, lua + "\n"), "Bea's Lua: " + lua);
        b.waitMs(2_000, "Lua waits for texts");
        openTerminal(b, ANN);
        clear(b, ANN);
        b.mutate(r -> hand(r, ANN).type(r, "python\n"), "Ann's Terminal: python");
        b.await(r -> after(hand(r, ANN).screen(r), "/ > python") instanceof String s && s.contains(">>>") ? null : "Ann's screen:\n" + hand(r, ANN).screen(r), "the Python prompt", 15_000);
        b.mutate(r -> hand(r, ANN).type(r, "import phone; print('sent', phone.sms_send('" + local(hand(r, BEA).number) + "', '" + SCRIPT_TEXT + "'))\n"),
                "Ann's Python: import phone; print('sent', phone.sms_send('<Bea>', '" + SCRIPT_TEXT + "'))");
        b.await(r -> {
            String s = hand(r, ANN).screen(r).replace("\n", "");
            if (s.contains("PhoneError") || s.contains("Traceback")) r.fail("Python: " + hand(r, ANN).screen(r));
            return s.matches("(?s).*sent \\d+.*") ? null : "Ann's screen:\n" + hand(r, ANN).screen(r);
        }, "Python: sms_send gives the message's id", 15_000);
        b.await(r -> {
            String s = hand(r, BEA).screen(r).replace("\n", "");
            return s.contains("lua got " + SCRIPT_TEXT + " from " + hand(r, ANN).number) ? null : "Bea's screen:\n" + hand(r, BEA).screen(r);
        }, "Lua's on_sms prints the text and Ann's number", 20_000);
        b.mutate(r -> hand(r, BEA).type(r, "\u0014"), "Ctrl+T stops the Lua script");
        b.timeLimit(120_000);
        return b.build();
    }

    // ------------------------------------------------------------ tel_ivr

    static Scenario ivr() {
        var b = start("tel_ivr", "Ann calls 611 on the Phone app's keypad, presses 2 on the in-call keypad, and gets a text with her number");
        network(b, List.of(ANN), List.of());
        openApp(b, ANN, "Phone", DIALER);
        dialOnKeypad(b, ANN, r -> "611", "611");
        b.await(r -> inCall(r, ANN), "611 answers: Ann is in the call", 15_000);
        b.waitMs(1_500, "the support line's greeting");
        b.await(r -> hand(r, ANN).tap(r, "Phone", "Keypad"), "Ann taps Keypad (in the call)", 5_000);
        b.await(r -> hand(r, ANN).shows(r, "Phone", "Hide keypad"), "the in-call keypad", 5_000);
        b.await(r -> hand(r, ANN).tap(r, "Phone", "2"), "Ann presses 2", 5_000);
        b.await(r -> {
            String why = notified(r, ANN, MESSAGES, "your number is");
            if (why != null) return why;
            var n = hand(r, ANN).notifications(r).stream().filter(x -> x.body().contains("your number is")).findFirst().orElseThrow();
            String d = digits(n.body());
            return d.contains(digits(hand(r, ANN).number).substring(1)) ? null : "the text says " + n.body();
        }, "a text from the support line: your number is Ann's", 25_000);
        b.await(r -> hand(r, ANN).tap(r, "Phone", "End"), "Ann taps End", 10_000);
        b.await(r -> inCall(r, ANN) == null ? "still in the call" : null, "the call ends", 10_000);
        b.timeLimit(100_000);
        return b.build();
    }

    static {
        for (Scenario s : List.of(register(), call(), callAsleep(), sms(), mmsGroup(), scripts(), ivr())) ALL.put(s.name, s);
    }
}
//?}
