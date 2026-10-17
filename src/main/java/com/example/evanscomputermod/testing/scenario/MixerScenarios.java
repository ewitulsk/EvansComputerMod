package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.speaker.AudioStream;
import com.example.evanscomputermod.speaker.SpeakerAudio;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerBlockEntity;
import com.example.evanscomputermod.speaker.SpeakerContent;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * The Volume Mixer (docs/gui/APPS.md "Volume Mixer"), as a player uses it from the console;
 * also the {@code ecm_apps} GameTest {@code volume_mixer}.
 *
 * <p>{@code volume_mixer}: a computer with a Speaker. Two programs play at once on it, each
 * its own stream on the speaker's mixer: {@code beep} (660 Hz) and {@code aplay} (a 440 Hz WAV
 * written from Python). {@code mixer} lists both strips with live meters; {@code mixer set beep
 * 30} turns beep down (its meter falls to about 30 %, aplay's stays); {@code mixer mute aplay}
 * silences aplay while beep keeps playing; the setting is remembered for the next run.
 * Control: {@code mixer mute master} silences the speaker (both strips still play, the master
 * meter is empty); {@code mixer unmute master} brings it back.
 */
public final class MixerScenarios {
    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final BlockPos PC = new BlockPos(0, 1, 0), SPEAKER = new BlockPos(-1, 1, 0);

    /** A 30 s 444 Hz square wave (+-9000, 16-bit) as a WAV, written by Python's audio module (no math module needed). */
    static final String MAKE_TONE = String.join("\n",
            "import audio",
            "half = bytes([0x28, 0x23]) * 9 + bytes([0xd8, 0xdc]) * 9",
            "audio.save_wav('/tone.wav', half * (8000 * 30 // 18), 8000)",
            "print('TONE written')",
            "");

    static final class State {
        int beepPeak, aplayPeak;
    }

    private static final Map<ScenarioRun, State> STATES = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    private static State st(ScenarioRun r) {
        return STATES.computeIfAbsent(r, k -> new State());
    }

    private MixerScenarios() {}

    private static SpeakerAudio card(ScenarioRun r) {
        return r.level().getBlockEntity(r.abs(SPEAKER)) instanceof SpeakerBlockEntity s ? s.audio() : null;
    }

    /** The live stream of program {@code name} on the Speaker (by its strip in the computer's mixer), or null. */
    private static AudioStream stream(ScenarioRun r, String name) {
        var c = AppScenarios.computer(r, "pc");
        SpeakerAudio card = card(r);
        if (c == null || card == null) return null;
        String text = c.volumeMixer().text();
        if (!text.contains("app " + name + " ")) return null;
        for (AudioStream s : card.streams()) {
            if (s.isClosed()) continue;
            String n = c.getProcessManager().nameOf(s.pid());
            if (n != null && (n.endsWith("/" + name + ".wasm") || n.equals(name + ".wasm") || n.equals(name))) return s;
        }
        return null;
    }

    private static String strip(ScenarioRun r, String app) {
        var c = AppScenarios.computer(r, "pc");
        if (c == null) return "no computer";
        for (String l : c.volumeMixer().text().split("\n")) if (l.startsWith("app " + app + " ")) return l;
        return "no strip for " + app + ": " + c.volumeMixer().text().replace('\n', ';');
    }

    private static Scenario volumeMixer() {
        var b = Scenario.builder("volume_mixer",
                        "two programs play on one Speaker; the mixer turns one down and mutes the other; master mute silences both")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .decor(PlayerKit.decor(List.of(SPEAKER), null, r -> r.player().place(SpeakerContent.SPEAKER.get(), r.abs(SPEAKER),
                        Direction.SOUTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH), null))
                .note("Setup (done for you): a Speaker placed west of the computer")
                .note("Write a 30 s 440 Hz WAV from Python (/mktone.py, typed in), then play it and a beep at the same time");
        PlayerKit.typeFiles(b, Map.of("pc", Map.of("/mktone.py", MAKE_TONE)));
        b.send("pc", "python /mktone.py");
        b.expect("pc", "^TONE written", "the WAV is written", "Error|Traceback");
        b.expect("pc", PlayerKit.PROMPT, "shell");
        b.send("pc", "aplay /tone.wav &");
        b.send("pc", "beep 660 30000 &");
        b.await(r -> {
            AudioStream a = stream(r, "aplay"), e = stream(r, "beep");
            if (a == null || e == null || a.peak() == 0 || e.peak() == 0) return "aplay " + a + ", beep " + e;
            st(r).aplayPeak = a.peak();
            st(r).beepPeak = e.peak();
            return null;
        }, "both play at once: two streams on the Speaker, each with its own strip and meter", 20_000);
        b.send("pc", "mixer");
        b.expect("pc", "aplay", "mixer lists the aplay strip", "^mixer: ");
        b.expect("pc", "beep", "and the beep strip", "^mixer: ");
        b.note("Turn beep down to 30 %");
        b.send("pc", "mixer set beep 30");
        b.await(r -> {
            AudioStream e = stream(r, "beep"), a = stream(r, "aplay");
            if (e == null || a == null) return "streams gone";
            int want = st(r).beepPeak * 30 / 100;
            return Math.abs(e.peak() - want) <= st(r).beepPeak / 10 && a.peak() >= st(r).aplayPeak * 9 / 10 ? null
                    : "beep peak " + e.peak() + " (want ~" + want + "), aplay " + a.peak() + "; " + strip(r, "beep");
        }, "beep's meter falls to about 30 %, aplay's stays", 10_000);
        b.note("Mute aplay");
        b.send("pc", "mixer mute aplay");
        b.await(r -> {
            AudioStream e = stream(r, "beep"), a = stream(r, "aplay");
            if (e == null || a == null) return "streams gone";
            return a.peak() == 0 && e.peak() > 0 && card(r).peak() > 0 ? null : "aplay " + a.peak() + ", beep " + e.peak() + ", speaker " + card(r).peak();
        }, "aplay is silent; beep still plays on the Speaker", 10_000);
        b.await(r -> strip(r, "aplay").contains("mute=1") && strip(r, "beep").contains("volume=30") ? null : strip(r, "aplay"),
                "the strips say so (and /var/lib/apps/mixer.conf keeps it for the next run)", 5_000);
        b.note("Control: mute the master strip");
        b.send("pc", "mixer mute master");
        b.await(r -> card(r).muted() && card(r).peak() == 0 && stream(r, "beep") != null ? null : "speaker peak " + card(r).peak(),
                "the Speaker is silent while beep still plays into it", 10_000);
        b.send("pc", "mixer unmute master");
        b.await(r -> !card(r).muted() && card(r).peak() > 0 ? null : "speaker peak " + card(r).peak(), "unmuted: beep is heard again", 10_000);
        b.send("pc", "mixer reset beep");
        b.send("pc", "mixer reset aplay");
        return b.timeLimit(60_000).build();
    }

    static {
        for (Scenario s : List.of(volumeMixer())) ALL.put(s.name, s);
    }
}
//?}
