package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.bluetooth.AirPodsData;
import com.example.evanscomputermod.bluetooth.AirPodsItem;
import com.example.evanscomputermod.bluetooth.BluetoothContent;
import com.example.evanscomputermod.bluetooth.Headsets;
import com.example.evanscomputermod.bluetooth.PhoneLink;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;
import com.example.evanscomputermod.voice.SoundField;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.Blocks;

import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * AirPods and the ePhone's Bluetooth (docs/BLUETOOTH.md), spawnable with
 * {@code /ecm scenario spawn airpods}; also the {@code ecm_bluetooth} GameTest.
 *
 * <p>{@code airpods}: a player holds an ePhone in the off hand and has AirPods and
 * their case. They put the AirPods on (right-click) and back into the case
 * (sneak + right-click the case). On the phone: Settings, Sections, Bluetooth.
 * Control: nothing to pair while the case is closed. They open the lid
 * (right-click the case): the phone lists "&lt;name&gt;'s AirPods ready to pair" (it
 * heard their advertising over the radio); Pair: paired and connected (the lid is
 * open), and control: the sound still plays on the phone's speaker (they are not
 * worn). Sneak + right-click the case: the AirPods go into the player's ears and the
 * phone says the sound plays on them. In the phone's Terminal, {@code beep}: the
 * sound travels to the AirPods over Bluetooth and plays in the wearer's ears
 * (control: nothing goes into the air from the phone's speaker for microphones).
 * A computer's Speaker beside the player beeps and the phone's {@code arecord}
 * records it loud through the AirPods' microphones (their frames reach the phone).
 * {@code bluetoothctl} shows the link. Then the AirPods go back into the case:
 * the phone's audio returns to its speaker.
 * <pre>
 *   Player at (0,1,0); computer (3,1,0) with a Speaker west of it at (2,1,0).
 * </pre>
 */
public final class AirPodsScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final BlockPos PC = new BlockPos(3, 1, 0), SPEAKER = new BlockPos(2, 1, 0);
    static final String SETTINGS = "dev.ecm.settings";
    /** A loud phone recording (peak -24 dBFS or above). */
    static final String LOUD = "arecord: recorded \\d+\\.\\d s, peak ?(?:0|-(?:[1-9]|1\\d|2[0-4])) dBFS";
    static final int AIRPODS_SLOT = 1, CASE_SLOT = 2;

    static {
        for (Scenario s : List.of(airpods())) ALL.put(s.name, s);
    }

    private AirPodsScenarios() {}

    static String podsName(ScenarioRun r) {
        return PhoneScenarios.holder(r).getGameProfile().getName() + "'s AirPods";
    }

    /** Null once the phone's Settings window shows a text containing {@code text}. */
    static String settingsShows(ScenarioRun r, String text) {
        Compositor c = PhoneGuiScenarios.comp(r);
        if (c == null) return "no phone GUI";
        for (var w : c.windows()) {
            if (!w.appId().equals(SETTINGS) || !w.mapped()) continue;
            var texts = c.textsOf(w.id());
            return texts.stream().anyMatch(t -> t.contains(text)) ? null : "Settings shows " + texts;
        }
        return "no Settings window";
    }

    static String settingsLacks(ScenarioRun r, String text) {
        return settingsShows(r, text) == null ? "Settings shows " + text : null;
    }

    /** Select hotbar slot {@code slot} and right-click the air with it (sneaking or not), as the player does. */
    static void useSlot(ScenarioRun r, int slot, boolean sneaking) {
        PhoneScenarios.select(PhoneScenarios.holder(r), slot);
        r.player().useHeld(sneaking);
    }

    /**
     * {@link PhoneScenarios#phoneRun} reading the phone's Terminal with its rows joined (it is 30
     * columns wide, so a long answer wraps).
     */
    static void phoneRun(Scenario.Builder b, String line, String re, String fail, String what, int timeoutMs) {
        java.util.regex.Pattern want = java.util.regex.Pattern.compile(re), bad = java.util.regex.Pattern.compile(fail);
        PhoneScenarios.clearScreen(b);
        b.mutate(r -> {
            if (!PhoneScenarios.type(r, PhoneScenarios.holder(r), line + "\n")) r.fail("the phone refused its holder's typing");
        }, "in the phone's Terminal: " + line);
        b.await(r -> {
            String joined = PhoneScenarios.screen(r).replace("\n", "");
            if (bad.matcher(joined).find()) {
                r.fail(what + ": the phone printed\n" + PhoneScenarios.screen(r));
                return null;
            }
            return want.matcher(joined).find() ? null : "phone screen:\n" + PhoneScenarios.screen(r);
        }, what, timeoutMs);
    }

    static PhoneLink link(ScenarioRun r) {
        PhoneHost h = PhoneScenarios.host(r);
        return h == null ? null : h.bluetooth().link();
    }

    private static Scenario airpods() {
        List<BlockPos> floor = PlayerKit.box(-3, 0, -3, 5, 0, 3);
        var b = Scenario.builder("airpods",
                        "pair AirPods with an ePhone from Settings > Bluetooth (case lid open), wear them: the phone's sound plays in your"
                                + " ears over Bluetooth and its microphone is the AirPods'; back in the case, the sound returns to the speaker")
                .asPlayer()
                .realTime()
                .host("pc", PC, "-")
                .decor(PlayerKit.decor(floor, r -> PlayerKit.fill(r, floor, Blocks.SMOOTH_STONE.defaultBlockState()), r -> {
                    r.player().place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER), Direction.SOUTH,
                            s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
                }, PhoneScenarios::cleanup))
                .note("Setup (done for you): a computer with a Speaker west of it, two blocks from where the player stands.")
                .mutate(r -> {
                    PhoneScenarios.givePhone(r);
                    com.example.evanscomputermod.phone.PhoneConfig.override("battery.enabled", false);
                    CellPhoneScenarios.phoneToOffHand(r);
                    var inv = PhoneScenarios.holder(r).getInventory();
                    inv.setItem(AIRPODS_SLOT, new ItemStack(BluetoothContent.AIRPODS.get()));
                    inv.setItem(CASE_SLOT, new ItemStack(BluetoothContent.AIRPODS_CASE.get()));
                }, "the player takes a new ePhone (off hand; batteries off for the run), AirPods (hotbar 2) and their case (hotbar 3)")
                .await(PhoneGuiScenarios::guiBooted, "the phone boots into its shell (held in the off hand)", 40_000);

        b.note("AirPods in the ears (right-click), then back into the case (sneak + right-click the case)");
        b.mutate(r -> useSlot(r, AIRPODS_SLOT, false), "right-click with the AirPods: they go into the player's ears");
        b.await(r -> PhoneScenarios.holder(r).getItemBySlot(EquipmentSlot.HEAD).getItem() instanceof AirPodsItem ? null : "not worn",
                "the AirPods are worn (head slot)", 2_000);
        b.mutate(r -> useSlot(r, CASE_SLOT, true), "sneak + right-click the case: the worn AirPods go into it");
        b.await(r -> AirPodsData.inCase(PhoneScenarios.holder(r).getInventory().getItem(CASE_SLOT)) != null
                        && PhoneScenarios.holder(r).getItemBySlot(EquipmentSlot.HEAD).isEmpty() ? null : "not in the case",
                "the AirPods are in the case", 2_000);

        b.note("Phone: Settings (home grid), Sections, Bluetooth");
        PhoneGuiScenarios.openApp(b, "Settings", SETTINGS);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Settings", "Sections"), "tap Sections", 10_000);
        b.await(r -> {
            int[] at = PhoneGuiScenarios.comp(r).locatePopupText("Bluetooth", false);
            if (at == null) return "no Sections menu yet";
            PhoneGuiScenarios.tap(r, at);
            return null;
        }, "tap Bluetooth in the menu", 10_000);
        b.await(r -> settingsShows(r, "MY DEVICES"), "Settings > Bluetooth: the phone's radio is on", 10_000);
        b.await(r -> settingsShows(r, "No devices paired yet."), "nothing paired yet", 5_000);
        b.await(r -> settingsLacks(r, "ready to pair"), "control: with the case closed, nothing to pair", 3_000);

        b.note("Open the case's lid (right-click it): the AirPods advertise; Settings lists them; Pair");
        b.mutate(r -> useSlot(r, CASE_SLOT, false), "right-click the case: the lid opens (pairing mode)");
        b.await(r -> settingsShows(r, podsName(r)) == null && settingsShows(r, "ready to pair") == null ? null
                        : settingsShows(r, "ready to pair") + " | " + Headsets.describe() + " | scan() " + (link(r) == null ? "-" : new com.example.evanscomputermod.bluetooth.BluetoothPeripheral(link(r)).scan()),
                "Settings lists the AirPods, ready to pair (heard over the radio)", 15_000);
        b.await(r -> CellPhoneScenarios.tapIn(r, "Settings", "Pair"), "tap Pair", 5_000);
        b.await(r -> settingsShows(r, "Connected"), "paired and connected (the lid is open)", 15_000);
        b.await(r -> settingsShows(r, "Sound plays on: this phone's speaker"), "control: not worn, so the sound stays on the phone's speaker", 5_000);
        b.await(r -> {
            ItemStack c = PhoneScenarios.holder(r).getInventory().getItem(CASE_SLOT);
            AirPodsData.Pods p = AirPodsData.inCase(c);
            PhoneLink l = link(r);
            return p != null && l != null && p.pairedTo() == l.addr ? null : "the AirPods don't remember the phone";
        }, "the AirPods remember the phone (paired to its address)", 2_000);

        b.note("Sneak + right-click the case: the AirPods go into the player's ears; the phone's sound moves to them");
        b.mutate(r -> useSlot(r, CASE_SLOT, true), "sneak + right-click the case: AirPods in the ears");
        b.await(r -> settingsShows(r, "Sound plays on: " + podsName(r)), "Settings: the sound plays on the AirPods", 10_000);

        b.note("Phone's Terminal: beep. The sound goes over Bluetooth to the AirPods and plays in the wearer's ears");
        b.mutate(r -> PhoneGuiScenarios.altKey(r, PhoneGuiScenarios.HID_H), "Alt+H: home");
        PhoneScenarios.openTerminal(b);
        b.mutate(r -> {
            if (!PhoneScenarios.type(r, PhoneScenarios.holder(r), "beep 660 3000 --wave sine &\n")) r.fail("the phone refused its holder's typing");
        }, "in the phone's Terminal: beep 660 3000 --wave sine &");
        b.await(r -> Headsets.earSamples(PhoneScenarios.holder(r).getUUID()) > 16_000 ? null
                        : "ear samples " + Headsets.earSamples(PhoneScenarios.holder(r).getUUID()),
                "the beep plays in the wearer's ears (AirPods audio frames received)", 10_000);
        b.await(r -> SoundField.get().find("phone:" + PhoneScenarios.st(r).phone) == null ? null : "the phone's speaker is in the air",
                "control: the phone's loudspeaker puts nothing in the air (microphones hear nothing of it)", 1_000);

        b.note("The computer's Speaker beside the player beeps; the phone records through the AirPods' microphones");
        b.send("pc", "beep 880 8000 --wave sine &");
        phoneRun(b, "arecord -d 1 /pods.wav", LOUD, "arecord: (?!recorded|/dev/mic)", "a loud recording through the AirPods", 20_000);
        b.await(r -> link(r).link() != null && link(r).link().micFrames > 0 ? null : "no microphone frames from the AirPods",
                "the recording came from the AirPods (their microphone frames reached the phone over Bluetooth)", 2_000);
        phoneRun(b, "bluetoothctl", "Audio: headphones", "bluetoothctl: ", "bluetoothctl shows the AirPods connected, sound on them", 10_000);

        b.note("Back into the case (sneak + right-click it): the link drops with them, the sound returns to the phone's speaker");
        b.mutate(r -> useSlot(r, CASE_SLOT, true), "sneak + right-click the case: the AirPods go back in");
        b.mutate(r -> useSlot(r, CASE_SLOT, false), "right-click the case: the lid closes");
        b.await(r -> link(r).connected() < 0 ? null : "still connected", "the AirPods hang up as the lid closes (the phone heard them go)", 5_000);
        phoneRun(b, "bluetoothctl", "Audio: speaker", "bluetoothctl: ", "bluetoothctl: the sound is back on the speaker", 15_000);
        return b.timeLimit(150_000).build();
    }

    /** Snapshot of what the scenario checks, for its log (tests). */
    static List<String> state(ScenarioRun r) {
        List<String> out = new ArrayList<>();
        PhoneLink l = link(r);
        if (l != null) out.add("connected " + l.connected() + " routed " + l.audioRouted());
        return out;
    }
}
//?}
