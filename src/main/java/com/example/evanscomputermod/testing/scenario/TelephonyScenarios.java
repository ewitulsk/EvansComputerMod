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
     * A carrier as the scenarios build it: its core and cell site computers (scenario nodes), the core's Card
     * Reader/Writer, the subscriber pool, a number block, the IMS pool and SIP domain, the backhaul addresses,
     * the site's id and LTE channel, and (for tel_interconnect) the core's address on the carriers' shared LAN.
     */
    record Carrier(String core, String site, BlockPos reader, String name, String pool, String block, String imsPool, String domain,
                   String coreIp, String siteIp, String siteId, int channel, String lanIp) {
        /** The block's E.164 prefix: 214-555 -> +1214555. */
        String prefix() {
            return "+1" + block.replace("-", "");
        }
    }

    static final Carrier RED = new Carrier("core", "site", READER, CARRIER, POOL, BLOCK, IMS_POOL, DOMAIN, CORE_IP, SITE_IP, "site-a", 1, "10.0.50.1");

    /** Which carrier's SIM each person gets. */
    static final Map<String, Carrier> HOME = new java.util.HashMap<>(Map.of(ANN, RED, BEA, RED, CAL, RED));

    /**
     * Everyone takes a new ePhone; the core (cored with a number block and IMS, imsd, smscd, mmscd, mrfd) and the site
     * come up; Ann writes a SIM per person ({@code cored sim new}: an IMSI and the block's next number) and hands
     * the others theirs and an LTE Modem; each fits modem and SIM in their phone; every phone attaches and registers
     * (VoLTE). {@code simOnly} people get a SIM (a number) but no phone.
     */
    static void network(Scenario.Builder b, List<String> people, List<String> simOnly) {
        takePhones(b, people, SPOT);
        coreUp(b, RED);
        imsUp(b, RED);
        siteUp(b, RED);
        phonesOn(b, people, simOnly, SPOT);
    }

    /** Ann takes a new ePhone into her off hand (it boots while she builds); the others take theirs and hold them. */
    static void takePhones(Scenario.Builder b, List<String> people, Map<String, Vec3> spot) {
        b.mutate(r -> {
            hand(r, ANN).givePhone(r, spot.get(ANN));
            hand(r, ANN).phoneToOffHand(r);
        }, "Ann takes a new ePhone (it boots while she builds) and puts it in her off hand");
        for (String p : people) {
            if (p.equals(ANN)) continue;
            b.mutate(r -> hand(r, p).givePhone(r, spot.get(p)), p + " takes a new ePhone and holds it, " + (int) spot.get(p).x
                    + " blocks east [exception: a second scenario player, docs/SCENARIO_EXCEPTIONS.md]");
        }
    }

    /** The backhaul's addresses; cored: the database, the number block, the IMS bearer, the site's credential, the core. */
    static void coreUp(Scenario.Builder b, Carrier c) {
        b.send(c.site(), "ifconfig eth0 " + c.siteIp() + "/24");
        b.send(c.core(), "ifconfig eth0 " + c.coreIp() + "/24");
        b.expect(c.core(), "eth0: inet " + Pattern.quote(c.coreIp()) + "/24", c.core() + " has " + c.coreIp());
        b.expect(c.site(), "eth0: inet " + Pattern.quote(c.siteIp()) + "/24", c.site() + " has " + c.siteIp());
        b.note(c.core() + ": the subscriber database, a number block, the IMS bearer, " + c.siteId() + "'s credential, the core");
        b.send(c.core(), "cored init " + c.name() + " " + c.pool());
        b.expectOrFail(c.core(), "^cored: carrier " + Pattern.quote(c.name()) + " pool " + Pattern.quote(c.pool()), "cored writes /etc/cored.db",
                "^cored: (?!carrier)");
        b.send(c.core(), "cored block add " + c.block());
        b.expectOrFail(c.core(), "^block " + c.block() + " added", "the number block " + c.block(), "^cored: ");
        b.send(c.core(), "cored ims pool=" + c.imsPool() + " domain=" + c.domain());
        b.expectOrFail(c.core(), "^ims pool " + Pattern.quote(c.imsPool()) + " pcscf \\S+ domain " + Pattern.quote(c.domain()),
                "the IMS bearer (VoLTE, texts)", "^cored: ");
        b.send(c.core(), "cored site-add " + c.siteId() + " " + CellularScenarios.secret(c.siteId()));
        b.expect(c.core(), "site " + Pattern.quote(c.siteId()) + " added", c.siteId() + "'s credential");
        b.send(c.core(), "cored start &");
        b.expect(c.core(), "cored: carrier " + Pattern.quote(c.name()) + " pool " + Pattern.quote(c.pool()) + ", cell0 ", "cored runs");
    }

    /** The IMS servers beside cored: imsd, smscd, mmscd, mrfd. */
    static void imsUp(Scenario.Builder b, Carrier c) {
        b.note(c.core() + ": the IMS servers beside cored (calls, texts, picture messages, voicemail and 611)");
        String d = Pattern.quote(c.domain());
        for (String[] s : new String[][]{{"imsd", "imsd: " + d + " on "}, {"smscd", "smscd: SMSC of " + d + " on "},
                {"mmscd", "mmscd: MMSC of " + d + " at "}, {"mrfd", "mrfd: media server of " + d + " on "}}) {
            b.send(c.core(), s[0] + " start &");
            b.expectOrFail(c.core(), s[1], s[0] + " runs", "^" + s[0] + ": (the HSS|can't|bind|no )");
        }
    }

    static void siteUp(Scenario.Builder b, Carrier c) {
        CellularScenarios.siteUp(b, c.site(), c.siteId(), c.name(), c.coreIp(), 1, PhyProfile.LTE, c.channel(), "");
    }

    /** A SIM per person at their carrier's core, the phones booted, modem and SIM in, attached and registered. */
    static void phonesOn(Scenario.Builder b, List<String> people, List<String> simOnly, Map<String, Vec3> spot) {
        List<String> all = new ArrayList<>(people);
        all.addAll(simOnly);
        for (String p : all) writeSim(b, HOME.get(p), p, people.contains(p));
        for (String p : people) b.await(r -> hand(r, p).guiBooted(r), p + "'s phone has booted into its shell", 40_000);
        b.mutate(r -> {
            Handset a = hand(r, ANN);
            a.standAt(r, spot.get(ANN));
            a.phoneToHand(r);
        }, "Ann walks back to her place, the phone back in her main hand");
        for (String p : people) {
            b.note(p + ": sneak-right-click the phone (its slot screen), shift-click the LTE Modem and the SIM into it");
            b.mutate(r -> hand(r, p).fitModemAndSim(r), p + "'s slot screen: LTE Modem and SIM in");
        }
        for (String p : people) {
            String carrier = HOME.get(p).name();
            b.await(r -> {
                PhoneUi u = hand(r, p).ui(r);
                return u != null && u.celldPid() > 0 && u.telephonydPid() > 0 ? null : p + ": events " + (u == null ? "-" : u.events());
            }, p + "'s phone starts celld and telephonyd by itself", 15_000);
            b.await(r -> hand(r, p).statusHas(r, "\ncarrier " + carrier + "\n"), p + "'s status bar shows " + carrier + " (attached)", 25_000);
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
    static void writeSim(Scenario.Builder b, Carrier c, String p, boolean phone) {
        String label = p.toLowerCase(java.util.Locale.ROOT);
        b.note(c.core() + ": a SIM for " + p + ". A blank card clicked into the Card Reader/Writer, cored sim new, taken out by hand");
        b.mutate(r -> CellularScenarios.insertBlank(r, c.reader()), "right-click " + c.core() + "'s Card Reader/Writer with a blank SIM card");
        b.send(c.core(), "cored sim new --label " + label);
        b.expectOrFail(c.core(), "^cored: wrote SIM IMSI \\d{15} \\(" + label + "\\), number " + Pattern.quote(c.prefix()) + "\\d{4}",
                "cored writes the SIM and gives it the block's next number", "^cored: (?!wrote|the card)");
        b.mutate(r -> {
            Matcher m = WROTE.matcher(r.screen(c.core()));
            String imsi = null, number = null;
            while (m.find()) if (m.group(2).equals(label)) {
                imsi = m.group(1);
                number = m.group(3);
            }
            if (imsi == null) {
                r.fail("no SIM for " + label + " on " + c.core() + "'s screen");
                return;
            }
            Handset h = hand(r, p);
            h.imsi = imsi;
            h.number = number;
            Inventory ann = r.player().entity().getInventory();
            ann.selected = 8;
            ann.setItem(8, ItemStack.EMPTY);
            r.player().useEmpty(r.abs(c.reader()), Direction.NORTH, false);
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
        }, "take the SIM out of the reader (empty hand)" + (p.equals(ANN) || !phone ? "" : " and hand it to " + p + " with an LTE Modem"
                + " [exception: items handed between players by moving them between inventories, docs/SCENARIO_EXCEPTIONS.md]"));
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
            b.mutate(r -> {
                hand(r, who).type(r, "tel calls\n");
                hand(r, who).typedAt = System.currentTimeMillis();
            }, who + "'s Terminal: tel calls (again every second until it shows)");
            b.await(r -> {
                Handset x = hand(r, who);
                String joined = x.screen(r).replace("\n", " ");
                Matcher m = VOICE.matcher(joined);
                long in = -1, out = -1;
                while (m.find()) {
                    in = Long.parseLong(m.group(1));
                    out = Long.parseLong(m.group(2));
                }
                String why;
                if (!joined.contains("active") || in < 0) why = who + "'s screen:\n" + x.screen(r);
                else if (n == 0 && (in == 0 || out == 0)) why = who + ": no voice yet:\n" + x.screen(r);
                else if (n == 1 && in <= x.mark) why = who + ": the counter didn't move (" + x.memo + " then " + in + "/" + out + ")";
                else {
                    if (n == 0) {
                        x.mark = in;
                        x.memo = in + "/" + out;
                    }
                    return null;
                }
                if (System.currentTimeMillis() - x.typedAt > 1_000) {
                    x.type(r, "tel calls\n");
                    x.typedAt = System.currentTimeMillis();
                }
                return why;
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
        // spaces left out: the Terminal wraps at a word, so a space can become a line break
        for (int i = 0; i < rows.length; i++) {
            joined.append(rows[i].replace(" ", ""));
            ends[i] = joined.length();
        }
        String want = line.replace(" ", "");
        int at = joined.lastIndexOf(want);
        if (at < 0) return null;
        int end = at + want.length(), row = 0;
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
        b.await(VideoScenarios::videoInstalled, "the server has video support (evanscomputermod-ffmpeg)", 1_000);
        b.mutate(r -> {
            try (java.io.InputStream in = VideoScenarios.class.getResourceAsStream(VideoScenarios.RESOURCES + VideoScenarios.VIDEO)) {
                if (in == null) throw new IllegalStateException(VideoScenarios.VIDEO + " is missing from the mod jar");
                java.nio.file.Path dir = hand(r, ANN).computer(r).storagePath().resolve("home/videos");
                java.nio.file.Files.createDirectories(dir);
                java.nio.file.Files.write(dir.resolve(VideoScenarios.VIDEO), in.readAllBytes());
            } catch (Exception e) {
                r.fail(e.toString());
            }
        }, "a video in Ann's phone's documents (/home/videos/" + VideoScenarios.VIDEO + ") [exception: the binary file is copied into the"
                + " phone's folder; docs/SCENARIO_EXCEPTIONS.md]");
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
        videoDuringCall(b);
        openTerminal(b, ANN);
        run(b, ANN, "tel calls", "active", null, "tel calls: the call is still up after the video", 10_000);
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

    /** Per run, the live-audio watch while a video plays: ticks seen, ticks whose speaker drain was live, samples, picture. */
    private static final Map<ScenarioRun, long[]> LIVE = Collections.synchronizedMap(new WeakHashMap<>());

    /** Ann's Video Player window (full screen), or null. */
    static Compositor.WindowInfo video(ScenarioRun r) {
        return hand(r, ANN).onScreen(r, VideoScenarios.APP);
    }

    static String videoTime(ScenarioRun r) {
        return VideoScenarios.timeText(hand(r, ANN).comp(r), video(r));
    }

    /**
     * Ann plays a video with sound (timed frames, the display pipeline's playout delay) while the call is up
     * and Bea talks. The call's sound must not wait on the picture: every speaker drain on Ann's phone that
     * carries Bea's voice is live (SpeakerAudio.lastDrainLive, sent with SpeakerAudioPacket.FLAG_LIVE at
     * once), while the video's time, picture and sound keep going.
     */
    static void videoDuringCall(Scenario.Builder b) {
        b.note("A video during the call: Ann opens the Video Player and plays a video while Bea talks; the call's"
                + " sound stays live, not on the picture's playout delay");
        b.mutate(r -> {
            PhoneUi u = hand(r, ANN).ui(r);
            if (u != null && u.foreground() != null) hand(r, ANN).altKey(r, PhoneGuiScenarios.HID_H);
        }, "Ann: Alt+H (home)");
        b.await(r -> {
            Compositor c = hand(r, ANN).comp(r);
            String label = c == null ? null : c.layerTexts("home").stream().filter(t -> t.startsWith("Video")).findFirst().orElse(null);
            return label == null ? "Ann's home grid " + (c == null ? "-" : c.layerTexts("home")) : hand(r, ANN).tapLayer(r, "home", label, false);
        }, "Ann: tap Video Player on the home grid", 20_000);
        b.await(r -> video(r) != null && hand(r, ANN).comp(r).textsOf(video(r).id()).contains(VideoScenarios.VIDEO) ? null
                : "Ann's windows " + hand(r, ANN).comp(r).windows(), "its library lists the video", 30_000);
        b.mutate(r -> {
            Compositor c = hand(r, ANN).comp(r);
            if (!hand(r, ANN).tapAt(r, c.locateText(video(r).id(), VideoScenarios.VIDEO, false))) r.fail("Ann's phone refused the touch");
        }, "Ann taps " + VideoScenarios.VIDEO);
        b.await(r -> video(r) != null && hand(r, ANN).comp(r).textsOf(video(r).id()).contains("Pause") && videoTime(r).startsWith("0:01") ? null
                : "Video Player: " + (video(r) == null ? "-" : hand(r, ANN).comp(r).textsOf(video(r).id())), "the video plays: its time runs", 15_000);
        b.mutate(r -> {
            var h = hand(r, ANN).host(r);
            LIVE.put(r, new long[]{0, 0, h.speakerAudio().samplesDrained(), VideoScenarios.pictureCrc(hand(r, ANN).comp(r), video(r)), System.currentTimeMillis()});
        }, "-");
        b.note("Bea talks for 3 s (a 440 Hz tone) [exception: a scenario player's voice is a test tone, docs/SCENARIO_EXCEPTIONS.md];"
                + " every tick, the last speaker drain of Ann's phone is checked");
        b.await(r -> {
            VoiceScenarios.talk(r, hand(r, BEA).other, 440, false);
            long[] s = LIVE.get(r);
            var h = hand(r, ANN).host(r);
            long ms = System.currentTimeMillis() - s[4];
            if (ms > 600) { // the first 0.6 s: the voice crosses the network and the jitter buffer
                s[0]++;
                if (h.speakerAudio().lastDrainLive()) s[1]++;
            }
            if (ms < 3_600) return "watching (" + s[1] + "/" + s[0] + " drains live)";
            long samples = h.speakerAudio().samplesDrained() - s[2];
            long crc = VideoScenarios.pictureCrc(hand(r, ANN).comp(r), video(r));
            String why = s[0] > 0 && s[1] * 10 >= s[0] * 9 ? null : "only " + s[1] + " of " + s[0] + " speaker drains were live while Bea talked";
            if (why == null && samples < 16_000) why = "Ann's speaker played only " + samples + " samples in 3 s";
            if (why == null && crc == s[3]) why = "the picture didn't move";
            if (why == null && !videoTime(r).matches("0:0[2-6].*")) why = "the video's time is " + videoTime(r);
            if (why != null) r.fail(why);
            r.say("§7  " + s[1] + " of " + s[0] + " speaker drains live; " + samples + " samples; video at " + videoTime(r));
            return null;
        }, "while the video plays, the call's sound goes out live (no playout delay) and the video keeps its time and picture", 6_000);
        b.mutate(r -> VideoScenarios.snapshot(r, hand(r, ANN).comp(r), "tel_call_video"), "snapshot: the video on Ann's phone during the call");
        b.note("Control: Bea stops talking and taps Mute (her microphone also hears Ann's phone 3 blocks away); the call is silent, so"
                + " Ann's speaker carries only the video's sound, which goes out on the picture's playout delay (not live)");
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Mute"), "Bea taps Mute on her in-call screen", 5_000);
        b.waitMs(800, "what was on its way plays out");
        b.mutate(r -> LIVE.put(r, new long[]{0, 0, hand(r, ANN).host(r).speakerAudio().samplesDrained(), 0, System.currentTimeMillis()}), "-");
        b.await(r -> {
            long[] s = LIVE.get(r);
            var h = hand(r, ANN).host(r);
            long ms = System.currentTimeMillis() - s[4];
            if (ms > 600) {
                s[0]++;
                if (h.speakerAudio().lastDrainLive()) s[1]++;
            }
            if (ms < 1_400) return "watching";
            long samples = h.speakerAudio().samplesDrained() - s[2];
            if (s[1] * 10 > s[0]) r.fail(s[1] + " of " + s[0] + " drains live with Bea silent");
            else if (samples < 4_000) r.fail("the video's sound stopped (" + samples + " samples)");
            else r.say("§7  silent call: " + s[1] + " of " + s[0] + " drains live; the video's sound plays (" + samples + " samples)");
            return null;
        }, "control: Bea silent, the video's sound alone is not live", 4_000);
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
            return u != null && u.ringingWho() != null ? null : "Bea's phone doesn't ring; wake locks " + h.computer(r).wakeLockHolders() + ", asleep " + h.computer(r).isAsleep() + "\ncelld.log:\n" + h.log(r, "var/log/celld.log", 800)
                    + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 800);
        }, "paged: Bea's phone wakes in her pocket and rings", 25_000);
        b.mutate(r -> hand(r, BEA).phoneToHand(r), "Bea takes the phone out (main hand)");
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Incoming call"), "Bea's phone shows the incoming call", 10_000);
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Answer"), "Bea taps Answer", 5_000);
        b.await(r -> inCall(r, BEA), "Bea is in the call", 10_000);
        run(b, ANN, "tel calls", "active", null, "Ann's tel calls: active", 10_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.note("A second call: Bea's phone back in her pocket. Once the network has been quiet a few seconds celld goes idle"
                + " again, so the next call is paged too (control for a phone that stayed connected after the first page)");
        b.mutate(r -> {
            Handset h = hand(r, BEA);
            h.movePhone(r, 9);
            h.inv(r).selected = 0;
        }, "Bea: the phone back into her pocket");
        b.await(r -> {
            PhoneHost h = hand(r, BEA).host(r);
            return h != null && h.state() == PhoneState.ASLEEP ? null : "Bea's phone is " + (h == null ? "-" : h.state());
        }, "Bea's phone is asleep again", 5_000);
        b.await(r -> {
            // the first idle was before the first call; a second one is this pocketing's
            String log = hand(r, BEA).log(r, "var/log/celld.log", 1_000_000);
            int n = log.split("going idle|idle again", -1).length - 1;
            return n >= 2 ? null : n + " idle line(s) in celld.log (end):\n" + log.substring(Math.max(0, log.length() - 600));
        }, "celld goes idle again (quiet since the call)", 20_000);
        run(b, ANN, r -> "tel dial " + local(hand(r, BEA).number), "tel dial <Bea>", "tel: calling", "^tel: (?!calling)", "Ann: tel dial Bea again", 10_000);
        b.await(r -> {
            Handset h = hand(r, BEA);
            PhoneHost ph = h.host(r);
            PhoneUi u = h.ui(r);
            if (ph == null || ph.state() != PhoneState.ASLEEP) return "Bea's phone is " + (ph == null ? "-" : ph.state());
            return u != null && u.ringingWho() != null ? null : "Bea's phone doesn't ring; wake locks " + h.computer(r).wakeLockHolders() + ", asleep " + h.computer(r).isAsleep() + "\ncelld.log:\n" + h.log(r, "var/log/celld.log", 2500) + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 1500);
        }, "paged again: the pocketed phone rings for the second call", 25_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.note("The phone never left the pocket this time: the page woke the session, and once it has been quiet celld goes idle"
                + " by itself, so a third call is paged as well");
        b.await(r -> hand(r, BEA).log(r, "var/log/celld.log", 1_000_000).contains("idle again") ? null
                        : "celld.log (end):\n" + hand(r, BEA).log(r, "var/log/celld.log", 600),
                "celld: idle again, quiet since the page", 20_000);
        run(b, ANN, r -> "tel dial " + local(hand(r, BEA).number), "tel dial <Bea>", "tel: calling", "^tel: (?!calling)", "Ann: tel dial Bea a third time", 10_000);
        b.await(r -> {
            Handset h = hand(r, BEA);
            PhoneHost ph = h.host(r);
            PhoneUi u = h.ui(r);
            if (ph == null || ph.state() != PhoneState.ASLEEP) return "Bea's phone is " + (ph == null ? "-" : ph.state());
            return u != null && u.ringingWho() != null ? null : "Bea's phone doesn't ring; wake locks " + h.computer(r).wakeLockHolders() + ", asleep " + h.computer(r).isAsleep() + "\ncelld.log:\n" + h.log(r, "var/log/celld.log", 2500) + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 1500);
        }, "paged a third time: still in the pocket, it rings", 25_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.timeLimit(180_000);
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
        b.note("Bea: Terminal, lua, a one-liner that prints each text phone.on_sms hands it and waits (phone.run); Ann: Terminal,"
                + " python, phone.sms_send to Bea's number; Lua prints the text and Ann's number. Ctrl+T ends the Lua script");
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
        b.note("Ann: Phone, Keypad, 611, Call. The carrier's support line (mrfd's IVR) answers; 1 texts the data used, 2 the number");
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

    // ------------------------------------------------------------ tel_interconnect

    static final String DAN = "Dan";
    static final BlockPos CORE_B = new BlockPos(16, 1, -6), READER_B = CORE_B.west(), SITE_B = FramedScenarios.B;
    /** The carriers' LAN switch (above the floor between the cores) and IANA's computer (the DNS root, ianad) south of it, its screen to the south. */
    static final BlockPos LAN_SW = new BlockPos(8, 2, -6), IANA = new BlockPos(8, 1, -4);
    static final String IANA_IP = "10.0.50.53", PEER_SECRET = "nni-red-blue-7f3a";
    static final Carrier BLUE = new Carrier("coreB", "siteB", READER_B, "cell-blue", "100.66.2.0/24", "972-555", "10.251.0.0/16", "blue.ecm",
            "10.0.41.1", "10.0.41.2", "site-b", 4, "10.0.50.2");
    /** Between the two sites: red's antenna (x 3) points east at them, blue's (x 13) west. */
    static final Map<String, Vec3> SPOT_TWO = Map.of(ANN, new Vec3(8.5, 0, 2.5), DAN, new Vec3(8.5, 0, -2.5));
    static final String TEXT_AB = "hello from red", TEXT_BA = "hi back from blue";

    static {
        HOME.put(DAN, BLUE);
    }

    static Scenario.Path backhaulB() {
        return Scenario.Path.from(CORE_B.below()).go(Direction.SOUTH, 6);
    }

    /** The root zone: the ecm. TLD delegated to IANA's own server (ianad adds e164.arpa). */
    static final String ROOT_ZONE = String.join("\n",
            "$TTL 300",
            "@        SOA ns.ecm. admin.ecm. 1 60 30 600 30",
            "@        NS  ns.ecm.",
            "ecm.     NS  ns.ecm.",
            "ns.ecm.  A   " + IANA_IP);
    /** ecm.: the two carriers' SIP domains (their borders on the LAN, A and SRV). */
    static final String ECM_ZONE = String.join("\n",
            "$ORIGIN ecm.",
            "$TTL 300",
            "@              SOA ns admin 1 60 30 600 30",
            "@              NS  ns",
            "ns             A   " + IANA_IP,
            "red            A   " + RED.lanIp(),
            "_sip._udp.red  SRV 10 0 5060 red",
            "blue           A   " + BLUE.lanIp(),
            "_sip._udp.blue SRV 10 0 5060 blue");

    static Scenario.Builder startInterconnect(String name, String what) {
        List<BlockPos> floor = new ArrayList<>(PlayerKit.box(-2, 0, -8, 24, 0, 6));
        floor.removeAll(backhaul().blocks());
        floor.removeAll(backhaulB().blocks());
        Scenario.Path lanA = Scenario.Path.from(LAN_SW.west()).go(Direction.WEST, 7);
        Scenario.Path lanB = Scenario.Path.from(LAN_SW.east()).go(Direction.EAST, 7);
        Scenario.Path lanIana = Scenario.Path.from(LAN_SW.south()).go(Direction.DOWN, 1);
        List<BlockPos> foot = new ArrayList<>(floor);
        foot.addAll(backhaul().blocks());
        foot.addAll(backhaulB().blocks());
        foot.addAll(lanA.blocks());
        foot.addAll(lanB.blocks());
        foot.addAll(lanIana.blocks());
        foot.addAll(FramedScenarios.station(SITE));
        foot.addAll(FramedScenarios.station(SITE_B));
        foot.add(READER);
        foot.add(READER_B);
        return Scenario.builder(name, what)
                .asPlayer()
                .realTime()
                .host("core", CORE, "-")
                .host("site", SITE, "-")
                .host("coreB", CORE_B, "-")
                .host("siteB", SITE_B, "-")
                .host("iana", IANA, "-", Direction.SOUTH)
                .switchNode("sw", LAN_SW)
                .link("backhaul", "core", Direction.DOWN, "site", Direction.DOWN, backhaul())
                .link("backhaulB", "coreB", Direction.DOWN, "siteB", Direction.DOWN, backhaulB())
                .link("lanA", "sw", Direction.WEST, "core", Direction.UP, lanA)
                .link("lanB", "sw", Direction.EAST, "coreB", Direction.UP, lanB)
                .link("lanIana", "sw", Direction.SOUTH, "iana", Direction.NORTH, lanIana)
                .decor(PlayerKit.decor(foot, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    FramedScenarios.build(r, SITE);
                    FramedScenarios.build(r, SITE_B);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(READER), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                    r.player().place(CardContent.CARD_READER.get(), r.abs(READER_B), Direction.NORTH,
                            s -> s.getValue(CardReaderBlock.FACING) == Direction.NORTH);
                }, TelephonyScenarios::cleanup))
                .note("Setup (done for you): two carriers side by side. cell-red: its core cabled (bottom) to its cell site, a Card"
                        + " Reader/Writer on the core's east side, the site's Advanced SDR, coax and LTE Sector Antenna pointing east."
                        + " cell-blue: the same mirrored 16 blocks east, its antenna pointing west, its reader on the core's west side."
                        + " The cores' tops are cabled to a switch, and so is IANA's computer south of it: the carriers' LAN.");
    }

    /** Per run, each carrier's ENUM key as IANA printed it (the carrier keeps it for porting in). */
    private static final Map<ScenarioRun, Map<String, String>> KEYS = Collections.synchronizedMap(new WeakHashMap<>());

    static final Pattern PORT_CODE = Pattern.compile("port-out code for \\+\\d+ is ([0-9a-f]{6}-[0-9a-f]{8})");

    /** {@code domain enum apply} at a carrier's core (its first block: IANA prints the ENUM key once), then {@code cored enum-key}. */
    static void enumApply(Scenario.Builder b, Carrier c) {
        b.note(c.core() + ": apply to IANA for block " + c.prefix() + " for " + c.domain() + "; IANA checks the domain resolves,"
                + " publishes the block's NAPTR and prints the carrier's ENUM key once; cored keeps it (for port-out codes)");
        b.send(c.core(), "domain --iana http://" + IANA_IP + "/ enum apply " + c.prefix() + " " + c.domain());
        b.expectOrFail(c.core(), "is held by " + Pattern.quote(c.domain()) + "; its ENUM key is", "IANA grants the block and prints the key", "^domain: ");
        b.sendFn(c.core(), r -> {
            Matcher m = Pattern.compile("its ENUM key is ([0-9a-f]{32})").matcher(r.screen(c.core()).replace("\n", ""));
            String key = null;
            while (m.find()) key = m.group(1);
            if (key != null) KEYS.computeIfAbsent(r, k -> new java.util.concurrent.ConcurrentHashMap<>()).put(c.domain(), key);
            return "cored enum-key " + (key == null ? "(no key on the screen)" : key);
        }, "cored enum-key <the key>");
        b.expectOrFail(c.core(), "enum key set", "cored has the carrier's ENUM key", "^cored: ");
    }

    /** {@code imsd peer add} (the other carrier's domain, its border on the LAN, the shared secret) and the border address. */
    static void peer(Scenario.Builder b, Carrier c, Carrier other) {
        b.send(c.core(), "imsd config set public_ip " + c.lanIp());
        b.expectOrFail(c.core(), "imsd: saved", c.core() + ": imsd's border is its LAN address", "^imsd: (?!saved)");
        b.send(c.core(), "imsd peer add " + other.domain() + " " + other.lanIp() + " " + PEER_SECRET);
        b.expectOrFail(c.core(), "imsd: peer " + Pattern.quote(other.domain()) + " added", c.core() + " peers with " + other.domain(), "^imsd: (?!peer)");
    }

    static Scenario interconnect() {
        var b = startInterconnect("tel_interconnect",
                "two carriers (cell-red 214-555 at red.ecm, cell-blue 972-555 at blue.ecm), each with its own core, IMS and site,"
                        + " cabled through a LAN switch with IANA's DNS root: ENUM blocks from ianad (domain enum apply), imsd peering with a"
                        + " shared secret; Ann (red) calls Dan (blue), they talk, she texts him and he texts back; control: blue drops the"
                        + " peer entry, Ann's next call is refused (403, call not allowed); then Dan ports his number to red (cored port-code, cored number ... ported,"
                        + " domain enum port, cored number release) and Ann's call to it rings him on red");
        takePhones(b, List.of(ANN, DAN), SPOT_TWO);
        coreUp(b, RED);
        coreUp(b, BLUE);

        b.note("The carriers' LAN: the switch's three ports as L2 ports in VLAN 1; the cores' tops and IANA's computer on 10.0.50.0/24");
        b.send("sw", "switch on").expect("sw", "Switch started", "switch running");
        b.send("sw", "switch");
        for (Direction face : new Direction[]{Direction.WEST, Direction.EAST, Direction.SOUTH}) {
            String p = b.eth("sw", face);
            b.send("sw", "interface " + p, "no routing");
            b.expect("sw", "converted to L2", p + " is an L2 port");
            b.send("sw", "exit");
        }
        for (String[] h : new String[][]{{"core", RED.lanIp()}, {"coreB", BLUE.lanIp()}, {"iana", IANA_IP}}) {
            String eth = b.eth(h[0], h[0].equals("iana") ? Direction.NORTH : Direction.UP);
            b.send(h[0], "ifconfig " + eth + " " + h[1] + "/24");
            b.expect(h[0], eth + ": inet " + Pattern.quote(h[1]) + "/24", h[0] + " has " + h[1] + " on the LAN");
        }

        b.note("IANA's computer: the root zone (ecm. delegated to itself), the ecm. zone with both carriers' domains, ianad's"
                + " settings; the cores' resolvers point at it");
        Map<String, Map<String, String>> files = new LinkedHashMap<>();
        files.put("iana", new java.util.TreeMap<>(Map.of(
                "/etc/named.conf", "zone . primary /etc/named/root.zone\nzone ecm primary /etc/named/ecm.zone",
                "/etc/named/root.zone", ROOT_ZONE,
                "/etc/named/ecm.zone", ECM_ZONE,
                "/etc/resolv.conf", "nameserver 127.0.0.1",
                "/etc/ianad.conf", "enum-ns ns.ecm")));
        files.put("core", Map.of("/etc/resolv.conf", "nameserver " + IANA_IP));
        files.put("coreB", Map.of("/etc/resolv.conf", "nameserver " + IANA_IP));
        PlayerKit.typeFiles(b, files);
        DnsScenarios.startNamed(b, "iana", "zone ecm\\.: loaded serial 1", "IANA's named serves . and ecm.");
        b.send("iana", "ianad &");
        b.expect("iana", "number block applications on port 80", "ianad takes applications (it adds e164.arpa to named)");
        b.send("core", "dig -t SRV _sip._udp.blue.ecm");
        b.expect("core", "_sip\\._udp\\.blue\\.ecm\\.\\s+\\d+ IN SRV\\s+10 0 5060 blue\\.ecm\\.", "red's core resolves blue's SIP border");

        enumApply(b, RED);
        enumApply(b, BLUE);
        b.send("core", "dig -t NAPTR " + enumName(BLUE.prefix()));
        b.expect("core", "NAPTR.*sip:.*@blue\\.ecm", "ENUM: blue's block points at blue.ecm (dig -t NAPTR)");

        b.note("Peering: each core lists the other carrier (its domain, its border on the LAN, the same secret), then the IMS servers start");
        peer(b, RED, BLUE);
        peer(b, BLUE, RED);
        imsUp(b, RED);
        imsUp(b, BLUE);
        siteUp(b, RED);
        siteUp(b, BLUE);
        phonesOn(b, List.of(ANN, DAN), List.of(), SPOT_TWO);

        b.note("Ann (red) calls Dan (blue): 10 digits, another area code; red's imsd finds blue.ecm by ENUM and sends the call to its border");
        openTerminal(b, ANN);
        run(b, ANN, r -> "tel dial " + hand(r, DAN).number.substring(2), "tel dial <Dan's 10 digits>", "tel: calling", "^tel: (?!calling)",
                "Ann: tel dial Dan", 10_000);
        b.await(r -> {
            Handset h = hand(r, DAN);
            PhoneUi u = h.ui(r);
            String w = h.shows(r, "Phone", "Incoming call");
            return w == null && u != null && u.ringingWho() != null ? null
                    : (w == null ? "Dan's phone doesn't ring" : w) + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 600);
        }, "Dan's phone rings: a call from the other carrier", 20_000);
        b.await(r -> hand(r, DAN).shows(r, "Phone", digits(hand(r, ANN).number).substring(4, 7)), "it shows Ann's number", 3_000);
        b.await(r -> hand(r, DAN).tap(r, "Phone", "Answer"), "Dan taps Answer", 5_000);
        b.await(r -> inCall(r, DAN), "Dan is in the call", 10_000);
        voiceFlows(b, ANN, "tel calls: the call to Dan is active, voice both ways through both carriers' relays");
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.await(r -> hand(r, DAN).shows(r, "Phone", "Call ended"), "Dan's Phone: Call ended", 10_000);
        b.mutate(r -> hand(r, ANN).phoneToOffHand(r), "Ann: the phone into her off hand (still awake), to type at the cores too");
        b.send("core", "cat /var/log/cdr/imsd.cdr");
        b.expect("core", "to=" + Pattern.quote(BLUE.prefix()) + "\\d{4}.*result=answered", "red's call record: to blue's number, answered");

        b.note("Texts both ways: Ann's SMSC hands the text to blue's border, blue's SMSC delivers it");
        run(b, ANN, r -> "sms send " + hand(r, DAN).number.substring(2) + " " + TEXT_AB, "sms send <Dan> " + TEXT_AB, "sms: sending to",
                "^sms: (?!sending)", "Ann: sms send", 10_000);
        b.await(r -> notified(r, DAN, MESSAGES, TEXT_AB), "Dan gets Ann's text", 30_000);
        openTerminal(b, DAN);
        run(b, DAN, r -> "sms send " + hand(r, ANN).number.substring(2) + " " + TEXT_BA, "sms send <Ann> " + TEXT_BA, "sms: sending to",
                "^sms: (?!sending)", "Dan: sms send", 10_000);
        b.await(r -> notified(r, ANN, MESSAGES, TEXT_BA), "Ann gets Dan's reply", 30_000);

        b.note("Control: blue removes red from its peers and restarts imsd; Ann's next call reaches blue's border, which refuses it (403)");
        b.send("coreB", "imsd peer remove red.ecm");
        b.expect("coreB", "imsd: peer red\\.ecm removed", "blue drops the peer entry");
        b.send("coreB", "imsd stop");
        b.expect("coreB", "stopping", "blue's imsd stops");
        b.waitMs(1_500, "it exits");
        b.send("coreB", "imsd start &");
        b.expectOrFail("coreB", "imsd: blue\\.ecm on .*0 peer\\(s\\)", "blue's imsd runs again, with no peers", "^imsd: (the HSS|can't|bind|no )");
        run(b, ANN, r -> "tel dial " + hand(r, DAN).number.substring(2), "tel dial <Dan>", "tel: calling", "^tel: (?!calling)", "Ann dials Dan again", 10_000);
        run(b, ANN, "tel log 1", "out .*call not allowed", null, "Ann's tel log: call not allowed (blue answered 403)", 20_000);
        b.send("coreB", "imsd log 5");
        b.expect("coreB", "INVITE from unlisted peer .*: 403", "blue's imsd log: the INVITE from an unlisted peer got 403");
        port(b);
        b.timeLimit(150_000);
        return b.build();
    }

    /**
     * Dan moves his number from blue to red: blue gives him a port-out code ({@code cored port-code}), red writes him a
     * SIM and gives it the number ({@code cored number ... ported}), red asks IANA to repoint the number
     * ({@code domain enum port} with the code and red's key), blue releases it; Dan swaps SIMs, his phone attaches to
     * red and registers with his old number, and Ann's call to that number now rings him on red.
     */
    static void port(Scenario.Builder b) {
        b.note("Porting: Dan takes his number from blue to red");
        b.sendFn("coreB", r -> "cored port-code " + hand(r, DAN).number, "cored port-code <Dan's number>");
        b.expectOrFail("coreB", "port-out code for " + Pattern.quote(BLUE.prefix()) + "\\d{4} is [0-9a-f]{6}-[0-9a-f]{8}",
                "blue prints Dan's port-out code", "^cored: (?!port-out)");
        b.mutate(r -> {
            Matcher m = PORT_CODE.matcher(r.screen("coreB").replace("\n", ""));
            String code = null;
            while (m.find()) code = m.group(1);
            Handset d = hand(r, DAN);
            d.memo = code == null ? "" : code;
            d.typed = d.number; // the number that moves
        }, "Dan notes the code (and his number)");
        b.note("Dan takes his blue SIM out; Ann writes him a red SIM and red gives it Dan's number (ported in)");
        b.mutate(r -> hand(r, DAN).simOut(r), "Dan: slot screen, the blue SIM out");
        writeSim(b, RED, DAN, true);
        b.mutate(r -> hand(r, DAN).number = hand(r, DAN).typed, "-");
        b.sendFn("core", r -> "cored number " + hand(r, DAN).imsi + " " + hand(r, DAN).number + " ported", "cored number <the new SIM> <Dan's number> ported");
        b.expectOrFail("core", "number " + Pattern.quote(BLUE.prefix()) + "\\d{4} \\(ported in\\)", "red gives the new SIM Dan's number", "^cored: ");
        b.sendFn("core", r -> "domain --iana http://" + IANA_IP + "/ enum port " + hand(r, DAN).number + " " + RED.domain() + " " + hand(r, DAN).memo
                + " " + KEYS.getOrDefault(r, Map.of()).getOrDefault(RED.domain(), "-"), "domain enum port <Dan's number> red.ecm <code> <red's key>");
        b.expectOrFail("core", "is ported to red\\.ecm \\(from blue\\.ecm\\)", "IANA points Dan's number at red.ecm", "^domain: ");
        b.sendFn("coreB", r -> "cored number release " + hand(r, DAN).number, "cored number release <Dan's number>");
        b.expectOrFail("coreB", "released " + Pattern.quote(BLUE.prefix()), "blue releases the number", "^cored: ");
        b.sendFn("core", r -> "dig -t NAPTR " + enumName(hand(r, DAN).number), "dig -t NAPTR <Dan's number's name>");
        b.expect("core", "NAPTR.*sip:.*@red\\.ecm", "ENUM: Dan's number now points at red.ecm (a record of its own over the block's)");
        b.mutate(r -> hand(r, DAN).simIn(r), "Dan: the red SIM in");
        b.await(r -> hand(r, DAN).statusHas(r, "\ncarrier " + RED.name() + "\n"), "Dan's phone attaches to red", 30_000);
        b.await(r -> volte(r, DAN), "Dan's phone registers on red with his old number", 30_000);
        b.note("Ann calls the number as before: red finds it is its own now (ported in) and rings Dan");
        run(b, ANN, r -> "tel dial " + hand(r, DAN).number.substring(2), "tel dial <Dan>", "tel: calling", "^tel: (?!calling)", "Ann dials Dan's number", 10_000);
        b.await(r -> hand(r, DAN).shows(r, "Phone", "Incoming call"), "Dan's phone rings, on red", 20_000);
        b.await(r -> hand(r, DAN).tap(r, "Phone", "Answer"), "Dan taps Answer", 5_000);
        b.await(r -> inCall(r, DAN), "Dan is in the call", 10_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.send("core", "imsd status");
        b.expect("core", "reg " + Pattern.quote(BLUE.prefix()) + "\\d{4} ", "red's imsd: the ported number is registered on red (the call stayed inside red)");
    }

    /** {@code +1972555} -> {@code 5.5.5.2.7.9.1.e164.arpa} (the block's ENUM name). */
    static String enumName(String prefix) {
        StringBuilder s = new StringBuilder();
        String d = prefix.replace("+", "");
        for (int i = d.length() - 1; i >= 0; i--) s.append(d.charAt(i)).append('.');
        return s + "e164.arpa";
    }

    // ------------------------------------------------------------ tel_tech_village

    /** The display phones of this village's Phone Store (their provisioned computers). */
    static java.util.Set<java.util.UUID> displayPhones(ScenarioRun r) {
        var s = CellVillageScenarios.st(r);
        var d = com.example.evanscomputermod.computer.WorldNetwork.get(r.level());
        java.util.Set<java.util.UUID> ids = new java.util.HashSet<>();
        for (int slot = 1; slot <= com.example.evanscomputermod.computer.CellPlan.PHONES; slot++) ids.add(d.identity(r.level(), s.a, "store.phone" + slot));
        return ids;
    }

    static int displayPhonesHeld(ScenarioRun r) {
        var ids = displayPhones(r);
        Inventory inv = r.player().entity().getInventory();
        int n = 0;
        for (int i = 0; i < inv.getContainerSize(); i++)
            if (inv.getItem(i).is(com.example.evanscomputermod.phone.PhoneContent.EPHONE.get())
                    && ids.contains(com.example.evanscomputermod.phone.PhoneItemData.id(inv.getItem(i)))) n++;
        return n;
    }

    /** Hit display frames (survival: the phone drops) and pick the phones up until Ann holds two of the store's phones. */
    static String takeTwoDisplayPhones(ScenarioRun r) {
        if (displayPhonesHeld(r) >= 2) return null;
        var s = CellVillageScenarios.st(r);
        BlockPos table = com.example.evanscomputermod.testing.TechCellChecks.store(s.va, com.example.evanscomputermod.testing.TechCellChecks.TABLE);
        CellVillageScenarios.follow(r, s, Vec3.atCenterOf(table));
        var box = new net.minecraft.world.phys.AABB(table).inflate(5, 3, 5);
        boolean dropped = !r.level().getEntitiesOfClass(net.minecraft.world.entity.item.ItemEntity.class, box,
                e -> e.getItem().is(com.example.evanscomputermod.phone.PhoneContent.EPHONE.get())).isEmpty();
        if (!dropped) {
            var frame = r.level().getEntitiesOfClass(net.minecraft.world.entity.decoration.ItemFrame.class, box).stream()
                    .filter(f -> f.getItem().is(com.example.evanscomputermod.phone.PhoneContent.EPHONE.get()))
                    .min(java.util.Comparator.comparingDouble(f -> f.distanceToSqr(Vec3.atCenterOf(table)))).orElse(null);
            if (frame == null) return "no phone left on the display table";
            Vec3 eye = frame.position().add(0, 1.5, 0).add(Vec3.atLowerCornerOf(s.va.datacenter().getRotation().rotate(Direction.SOUTH).getNormal()).scale(1.5));
            r.player().lookFrom(eye, frame.position());
            var p = r.player().entity();
            p.setGameMode(net.minecraft.world.level.GameType.SURVIVAL);
            try {
                p.attack(frame);
            } finally {
                p.setGameMode(net.minecraft.world.level.GameType.CREATIVE);
            }
            r.say("§7  hit the display frame at " + frame.blockPosition().toShortString() + ": the phone drops");
            return "picking it up";
        }
        String why = r.player().pickUp(table.above(), com.example.evanscomputermod.phone.PhoneContent.EPHONE.get());
        return why != null ? why : displayPhonesHeld(r) + " display phone(s) held";
    }

    /**
     * Ann holds one display phone and uses it (it keeps the computer generation provisioned for it); the other goes to
     * Bea [exception: items handed between players], who holds and uses hers. Both stand in the store.
     */
    static void holdDisplayPhones(ScenarioRun r) {
        var ids = displayPhones(r);
        Inventory inv = r.player().entity().getInventory();
        List<Integer> slots = new ArrayList<>();
        for (int i = 0; i < inv.getContainerSize(); i++)
            if (ids.contains(com.example.evanscomputermod.phone.PhoneItemData.id(inv.getItem(i)))) slots.add(i);
        if (slots.size() < 2) {
            r.fail("Ann holds " + slots.size() + " display phones");
            return;
        }
        com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
        Handset a = hand(r, ANN), bea = hand(r, BEA);
        ItemStack first = inv.getItem(slots.get(0)), second = inv.getItem(slots.get(1));
        inv.setItem(slots.get(0), ItemStack.EMPTY);
        inv.setItem(slots.get(1), ItemStack.EMPTY);
        inv.setItem(0, first);
        inv.selected = 0;
        com.example.evanscomputermod.phone.PhoneServer.track(r.player().entity());
        r.player().useHeld(false);
        a.phone = com.example.evanscomputermod.phone.PhoneItemData.id(inv.getItem(0));
        PhoneScenarios.st(r).phone = a.phone;
        bea.standAt(r, new Vec3(1.5, 0, 0.5));
        Inventory bi = bea.inv(r);
        bi.setItem(0, second);
        bi.selected = 0;
        com.example.evanscomputermod.phone.PhoneServer.track(bea.other.entity());
        bea.other.useHeld(false);
        bea.phone = com.example.evanscomputermod.phone.PhoneItemData.id(bi.getItem(0));
        if (a.phone == null || bea.phone == null || !ids.contains(a.phone) || !ids.contains(bea.phone)) {
            r.fail("using the display phones did not keep their computers: " + a.phone + ", " + bea.phone);
            return;
        }
        r.say("§7  Ann holds display phone " + a.phone.toString().substring(0, 8) + ", Bea " + bea.phone.toString().substring(0, 8));
    }

    static final Pattern STATUS_NUMBER = Pattern.compile("\nnumber (\\+\\d{11})\n");

    /** Null once {@code who}'s phone is registered (VoLTE) with a number of the store's lines (NPA-555-0100 to 0103); keeps the number. */
    static String storeLine(ScenarioRun r, String who) {
        Handset h = hand(r, who);
        String st = h.status(r);
        Matcher m = STATUS_NUMBER.matcher(st);
        if (!st.contains("\nvolte 1\n") || !m.find()) return who + "'s uictl phone:\n" + st + "\ntelephonyd.log:\n" + h.log(r, "var/log/telephonyd.log", 800);
        h.number = m.group(1);
        if (!h.number.matches("\\+1\\d{3}555010[0-3]")) {
            r.fail(who + "'s store phone has number " + h.number + ", not one of the display lines 0100-0103");
            return null;
        }
        return null;
    }

    static final String VILLAGE_TEXT = "meet at the data center";

    static Scenario techVillage() {
        var b = Scenario.builder("tel_tech_village",
                        "telephony in a Tech Village: two phones from a Phone Store's display (numbered lines of the village's block) attach"
                                + " and register with the core's IMS; Ann calls Bea and they talk, she texts her, and she calls the carrier's 611"
                                + " (press 2: a text with her number)")
                .asPlayer()
                .realTime()
                .inWorld()
                .timeLimit(240_000);
        b.note("Stand in a Tech Village's Phone Store. Built by the world: the store's cell site, the display phones (each with an LTE"
                + " Modem and a SIM numbered from the village's block: lines 0100-0103), and the carrier core in one village's data center,"
                + " which runs imsd, smscd, mmscd and mrfd beside cored (services.cfg). Bea is a second player"
                + " [exception: a second scenario player, docs/SCENARIO_EXCEPTIONS.md]");
        b.await(r -> CellVillageScenarios.st(r).va.datacenter() == null ? "no Phone Store in village " + CellVillageScenarios.st(r).a : null,
                "find this village's Phone Store", 5_000);
        b.mutate(r -> {
            Inventory inv = r.player().entity().getInventory();
            for (int i = 0; i < inv.getContainerSize(); i++)
                if (inv.getItem(i).is(com.example.evanscomputermod.phone.PhoneContent.EPHONE.get())) inv.setItem(i, ItemStack.EMPTY);
        }, "Ann puts away the phones she carries (creative inventory)");
        b.note("Take two phones from the display table: hit their item frames, pick them up; hand one to Bea");
        b.await(TelephonyScenarios::takeTwoDisplayPhones, "two display phones are in Ann's inventory", 20_000);
        b.mutate(TelephonyScenarios::holdDisplayPhones, "Ann holds one and uses it; Bea gets the other, holds it and uses it"
                + " [exception: items handed between players by moving them between inventories, docs/SCENARIO_EXCEPTIONS.md]");
        for (String p : List.of(ANN, BEA)) b.await(r -> hand(r, p).guiBooted(r), p + "'s phone has booted into its shell", 60_000);
        for (String p : List.of(ANN, BEA)) {
            b.await(r -> {
                PhoneUi u = hand(r, p).ui(r);
                return u != null && u.celldPid() > 0 && u.telephonydPid() > 0 ? null : p + ": events " + (u == null ? "-" : u.events());
            }, p + "'s phone starts celld and telephonyd (its modem and SIM are in)", 20_000);
            b.await(r -> hand(r, p).statusHas(r, "\ncarrier " + CellVillageScenarios.plan(r).carrier + "\n"), p + "'s phone attaches to the store's cell site", 30_000);
            b.await(r -> storeLine(r, p), p + "'s phone registers with the core's IMS: VoLTE and its store line's number", 40_000);
        }
        b.note("Ann calls Bea (7 digits: the same area code)");
        openTerminal(b, ANN);
        run(b, ANN, r -> "tel dial " + local(hand(r, BEA).number), "tel dial <Bea>", "tel: calling", "^tel: (?!calling)", "Ann: tel dial Bea", 10_000);
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Incoming call"), "Bea's phone rings: the incoming-call screen", 25_000);
        b.await(r -> hand(r, BEA).tap(r, "Phone", "Answer"), "Bea taps Answer", 5_000);
        b.await(r -> inCall(r, BEA), "Bea is in the call", 10_000);
        voiceFlows(b, ANN, "tel calls: the call is active, voice both ways through the core's relay");
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.await(r -> hand(r, BEA).shows(r, "Phone", "Call ended"), "Bea's Phone: Call ended", 10_000);
        b.note("Ann texts Bea");
        run(b, ANN, r -> "sms send " + local(hand(r, BEA).number) + " " + VILLAGE_TEXT, "sms send <Bea> " + VILLAGE_TEXT, "sms: sending to",
                "^sms: (?!sending)", "Ann: sms send", 10_000);
        b.await(r -> notified(r, BEA, MESSAGES, VILLAGE_TEXT), "Bea gets the text (a notification)", 30_000);
        b.note("Ann calls 611 (the store's sign says so), presses 2: the support line texts her number");
        run(b, ANN, "tel dial 611", "tel: calling", "^tel: (?!calling)", "Ann: tel dial 611", 10_000);
        run(b, ANN, "tel calls", "active", null, "611 answers", 20_000);
        b.waitMs(1_500, "the support line's greeting");
        run(b, ANN, "tel dtmf 2", PROMPT, "^tel: ", "Ann presses 2 (tel dtmf 2)", 10_000);
        b.await(r -> {
            String why = notified(r, ANN, MESSAGES, "your number is");
            if (why != null) return why;
            var n = hand(r, ANN).notifications(r).stream().filter(x -> x.body().contains("your number is")).findFirst().orElseThrow();
            return digits(n.body()).contains(digits(hand(r, ANN).number).substring(1)) ? null : "the text says " + n.body();
        }, "a text from the support line: your number is Ann's", 30_000);
        run(b, ANN, "tel hangup", PROMPT, "^tel: ", "Ann hangs up", 10_000);
        b.mutate(r -> {
            Inventory inv = r.player().entity().getInventory();
            for (int i = 0; i < inv.getContainerSize(); i++)
                if (inv.getItem(i).is(com.example.evanscomputermod.phone.PhoneContent.EPHONE.get())) inv.setItem(i, ItemStack.EMPTY);
            hand(r, BEA).forget(r);
            CellVillageScenarios.release(r);
        }, "Ann and Bea put their phones away (the ticking area around the player ends)");
        return b.build();
    }

    static {
        for (Scenario s : List.of(register(), call(), callAsleep(), sms(), mmsGroup(), scripts(), ivr(), interconnect(), techVillage())) ALL.put(s.name, s);
    }
}
//?}
