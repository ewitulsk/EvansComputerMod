package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.handheld.HandheldBand;
import com.example.evanscomputermod.radio.handheld.HandheldPackets;
import com.example.evanscomputermod.radio.handheld.HandheldServer;
import com.example.evanscomputermod.radio.handheld.HandheldSettings;
import com.example.evanscomputermod.radio.handheld.RadioHandheldContent;
import com.example.evanscomputermod.radio.sdr.RadioSdrContent;
import com.example.evanscomputermod.radio.sdr.SdrBlock;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import com.example.evanscomputermod.speaker.SpeakerBlock;
import com.example.evanscomputermod.speaker.SpeakerContent;
import com.example.evanscomputermod.voice.MicrophoneBlock;
import com.example.evanscomputermod.voice.MicrophoneBlockEntity;
import com.example.evanscomputermod.voice.VoiceContent;
import com.example.evanscomputermod.voice.VoicePackets;
import com.example.evanscomputermod.voice.VoiceServer;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.InteractionHand;
import net.minecraft.world.item.ItemStack;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.WeakHashMap;

/**
 * Microphones and voice over radio (docs/VOICE.md), spawnable with
 * {@code /ecm scenario spawn <name>}; also the {@code ecm_voice} GameTests.
 *
 * <p>{@code mic_record}: a Terminal with a Speaker the player puts west of it
 * and a Microphone east of it (two blocks from the Speaker). Control: with
 * the room quiet, {@code arecord -d 1} records only the capsule's hiss (peak
 * -50 dBFS or lower). Then the Speaker beeps in the background and
 * {@code arecord} records it loud (the Microphone's tally lamp lights while it
 * records and goes out after); {@code mic_check.py} and {@code mic_check.lua}
 * read the same Microphone through Python's and Lua's {@code audio.open_mic}.
 * <pre>
 *   Terminal (0,1,0) facing north, Speaker (-1,1,0), Microphone (1,1,0).
 * </pre>
 *
 * <p>{@code mic_voice}: the same Terminal and Microphone; the player stands two
 * blocks from the Microphone and talks. Control: nothing asks for their
 * voice before a program records. While {@code arecord} runs, the server asks
 * the player's client for their voice, and the recording is loud. Control:
 * from 40 blocks away the Microphone hears only hiss. [Exception: a scenario
 * player has no microphone; its voice is a 440 Hz tone sent through the same
 * server handler the client's voice packets go to.]
 *
 * <p>{@code radio_voice}: two players with Walkie-Talkies 24 blocks apart,
 * both switched on (right-click) and tuned to UHF 462.5625 MHz (the tuning
 * screen's settings); the talker binds push-to-talk to Caps Lock on the
 * tuning screen. The talker holds PTT and speaks: the listener's radio plays
 * the talker's 700 Hz voice, and the talker's own radio plays nothing while it
 * talks (half duplex). Controls: nothing before PTT is held, nothing on the
 * next channel (462.5875 MHz), and nothing from a receive-only Handheld Radio
 * with PTT held. Then a computer talks: a Terminal with a Standard SDR, a
 * Microphone and a Speaker runs {@code radio_station /dev/mic 462.5625M}
 * while its Speaker beeps 1000 Hz, and the listener's radio plays the 1000 Hz. [Exceptions: the scenario player's
 * voice as in mic_voice; the handhelds' per-player ticks are run by the
 * scenario, since the server never ticks scenario players.]
 * <pre>
 *   Talker at (0,1,0); listener at (0,1,24). Station Terminal (4,1,0): Standard SDR (5,1,0),
 *   Microphone (3,1,0), Speaker (4,1,1).
 * </pre>
 */
public final class VoiceScenarios {

    public static final Map<String, Scenario> ALL = new LinkedHashMap<>();

    static final BlockPos PC = new BlockPos(0, 1, 0);
    static final BlockPos SPEAKER = new BlockPos(-1, 1, 0);
    static final BlockPos MIC = new BlockPos(1, 1, 0);

    static final BlockPos STATION = new BlockPos(4, 1, 0), STATION_SDR = new BlockPos(5, 1, 0),
            STATION_MIC = new BlockPos(3, 1, 0), STATION_SPEAKER = new BlockPos(4, 1, 1);
    static final BlockPos TALKER = new BlockPos(0, 1, 3), LISTENER = new BlockPos(0, 1, 27);
    static final double FREQ_HZ = 462.5625e6, OFF_HZ = 462.5875e6;

    /** A loud recording: peak at -24 dBFS or above. */
    static final String LOUD = "^arecord: recorded \\d+\\.\\d s, peak (?:0|-(?:[1-9]|1\\d|2[0-4])) dBFS, overruns 0$";
    /** A quiet one: peak at -50 dBFS or below. */
    static final String QUIET = "^arecord: recorded \\d+\\.\\d s, peak -(?:[5-9]\\d) dBFS";

    static {
        for (Scenario s : List.of(record(), voice(), radio())) ALL.put(s.name, s);
    }

    private VoiceScenarios() {
    }

    // ------------------------------------------------------------ mic_record

    private static Scenario record() {
        var b = Scenario.builder("mic_record",
                        "a Microphone next to a computer records what a Speaker plays (arecord, Python and Lua audio.open_mic)")
                .host("pc", PC, "-")
                .asPlayer()
                .decor(PlayerKit.decor(List.of(SPEAKER, MIC), null, VoiceScenarios::placeSpeakerAndMic, null))
                .note("Setup (done for you): a Speaker west of the computer, a Microphone east of it (2 blocks apart)")
                .note("Control: the room is quiet; the Microphone hears only its own hiss")
                .send("pc", "arecord -d 1 /quiet.wav")
                .expect("pc", QUIET, "a quiet recording (peak -50 dBFS or lower)", "^arecord: (?!recorded|/dev/mic)")
                .expect("pc", PlayerKit.PROMPT, "arecord ends")
                .await(r -> lit(r) ? "still lit" : null, "control: the tally lamp goes out when nothing records", 5_000)
                .note("Beep the Speaker in the background and record it")
                .send("pc", "beep 880 6000 --wave sine &")
                .send("pc", "arecord -d 2 /tone.wav")
                .await(r -> lit(r) ? null : "the tally lamp is off", "the Microphone's tally lamp lights while it records", 5_000)
                .expect("pc", LOUD, "a loud recording of the beep", "^arecord: recorded .* peak -(?:2[5-9]|[3-9]\\d) dBFS")
                .expect("pc", PlayerKit.PROMPT, "arecord ends")
                .note("The same Microphone from Python and Lua (audio.open_mic), while the Speaker beeps again");
        typeBundled(b, Map.of("/mic_check.py", "mic_check.py", "/mic_check.lua", "mic_check.lua"));
        return b.send("pc", "beep 660 8000 --wave sine &")
                .send("pc", "python mic_check.py")
                .expect("pc", "^PYMIC /dev/mic 16000 (?:0|-(?:[1-9]|1\\d|2[0-4]))$", "Python records half a second, loud",
                        "^(?:Traceback|PYMIC .* -(?:2[5-9]|[3-9]\\d)$)")
                .expect("pc", PlayerKit.PROMPT, "the script ends")
                .send("pc", "lua mic_check.lua")
                .expect("pc", "^LUAMIC\\s+/dev/mic\\s+16000\\s+(?:0|-(?:[1-9]|1\\d|2[0-4]))$", "Lua records half a second, loud",
                        "^(?:lua: |LUAMIC.*-(?:2[5-9]|[3-9]\\d)$)")
                .expect("pc", PlayerKit.PROMPT, "the script ends")
                .timeLimit(90_000)
                .build();
    }

    // ------------------------------------------------------------ mic_voice

    private static Scenario voice() {
        return Scenario.builder("mic_voice",
                        "a Microphone next to a computer records a player talking two blocks away (and not from 40 blocks)")
                .host("pc", PC, "-")
                .asPlayer()
                .realTime()
                .decor(PlayerKit.decor(List.of(MIC), null, VoiceScenarios::placeMic, null))
                .note("Setup (done for you): a Microphone east of the computer")
                .note("Turn on Microphone input in Mods > Evans Computer Mod > Config, stand two blocks from the Microphone and"
                        + " talk (hold Talk, B, in push-to-talk mode) [exception: the scenario player's voice is a 440 Hz tone sent"
                        + " through the server's voice packet handler; docs/SCENARIO_EXCEPTIONS.md]")
                .await(r -> VoiceServer.demandFor(speaker(r)) == 0 ? null : "something already wants the voice",
                        "control: nothing asks for the player's voice before a program records", 2_000)
                .send("pc", "arecord -d 2 /voice.wav")
                .mutate(r -> stand(r, r.player(), MIC.offset(2, 0, 0)), "step away from the keyboard: two blocks east of the Microphone")
                .await(r -> {
                    talk(r, r.player(), 440, false);
                    return (VoiceServer.demandFor(speaker(r)) & VoicePackets.DEMAND_MIC) != 0 ? null : "no demand yet";
                }, "while arecord runs, the server asks the player's client for their voice", 5_000)
                .await(r -> talkFor(r, r.player(), 440, 60), "the player talks for three seconds", 10_000)
                .expect("pc", LOUD, "a loud recording of the player's voice", "^arecord: recorded .* peak -(?:2[5-9]|[3-9]\\d) dBFS")
                .expect("pc", PlayerKit.PROMPT, "arecord ends")
                .note("Control: walk 40 blocks away and talk; the Microphone hears only hiss")
                .send("pc", "arecord -d 1 /far.wav")
                .mutate(r -> stand(r, r.player(), MIC.offset(40, 0, 0)), "walk 40 blocks east")
                .await(r -> talkFor(r, r.player(), 440, 40), "the player talks for two seconds", 10_000)
                .expect("pc", QUIET, "a quiet recording", "^arecord: recorded .* peak (?:0|-(?:[1-9]|[1-4]\\d)) dBFS")
                .expect("pc", PlayerKit.PROMPT, "arecord ends")
                .timeLimit(60_000)
                .build();
    }

    // ------------------------------------------------------------ radio_voice

    private static Scenario radio() {
        return Scenario.builder("radio_voice",
                        "talk on a Walkie-Talkie (push-to-talk, half duplex, FM on the air) and hear it on another 24 blocks away;"
                                + " then a computer talks on the same channel through radio_station /dev/mic")
                .host("station", STATION, "-")
                .asPlayer()
                .realTime()
                .decor(PlayerKit.decor(List.of(STATION_SDR, STATION_MIC, STATION_SPEAKER), null, VoiceScenarios::buildStation, null))
                .note("Setup (done for you): a station computer with a Standard SDR, a Microphone and a Speaker")
                .note("Two players take a Walkie-Talkie each, right-click to switch it on and tune it (sneak + right-click) to UHF"
                        + " 462.5625 MHz [exceptions: voice tones through the voice packet handler, and the radios' per-player ticks run"
                        + " by the scenario; docs/SCENARIO_EXCEPTIONS.md]")
                .mutate(VoiceScenarios::handOutRadios, "the talker and the listener take their radios, switch them on and tune 462.5625 MHz")
                .mutate(VoiceScenarios::bindPtt, "the talker binds push-to-talk to Caps Lock on the tuning screen")
                .await(r -> listen(r, 700, false, 40), "control: before anyone talks, no voice on the channel", 20_000)
                .mutate(r -> check(r, "silent", false), "the listener's radio plays no voice")
                .note("The talker holds PTT and speaks")
                .mutate(VoiceScenarios::markTalkerAudio, "(note what the talker's radio has played so far)")
                .await(r -> listen(r, 700, true, 80), "the talker talks for four seconds", 20_000)
                .mutate(r -> check(r, "talk", true), "the listener's radio plays the talker's 700 Hz voice")
                .mutate(VoiceScenarios::checkHalfDuplex, "the talker's own radio played nothing while it talked (half duplex)")
                .note("The talker speaks quietly (-30 dBFS): the radio's mic ALC still modulates fully")
                .await(r -> listen(r, 700, true, 60, 0.0316), "the talker talks quietly for three seconds", 20_000)
                .mutate(r -> check(r, "quiet", true), "the listener hears the quiet voice at a normal level")
                .note("Control: the listener tunes to the next channel, 462.5875 MHz, while the talker keeps talking")
                .mutate(r -> tune(r, listener(r), OFF_HZ), "the listener tunes to 462.5875 MHz")
                .await(r -> listen(r, 700, true, 60), "the talker talks for three seconds", 20_000)
                .mutate(r -> check(r, "off", false), "nothing on 462.5875 MHz")
                .note("Control: the talker swaps to a receive-only Handheld Radio on the same channel and holds PTT")
                .mutate(r -> {
                    tune(r, listener(r), FREQ_HZ);
                    VoiceServer.onTalk(speaker(r), false);
                    holdRadio(r, r.player(), RadioHandheldContent.HANDHELD_RADIO.get());
                }, "the listener tunes back; the talker takes a Handheld Radio, switches it on and tunes 462.5625 MHz")
                .await(r -> listen(r, 700, true, 60), "the talker talks for three seconds", 20_000)
                .mutate(r -> check(r, "receiver", false), "a Handheld Radio doesn't transmit")
                .mutate(r -> VoiceServer.onTalk(speaker(r), false), "the talker lets go of PTT")
                .note("Now the computer talks: its Speaker beeps 1000 Hz next to its Microphone, and radio_station puts the"
                        + " Microphone on the air")
                .send("station", "beep 1000 9000 --wave sine &")
                .send("station", "radio_station /dev/mic 462.5625M --power 0 --duration 6 &")
                .expect("station", "radio_station: live from /dev/mic on ", "radio_station goes live", "radio_station: (?!live|done)")
                .await(r -> listen(r, 1000, false, 80), "the listener listens for four seconds", 20_000)
                .mutate(r -> check(r, "computer", true), "the listener's radio plays the computer's 1000 Hz")
                .timeLimit(120_000)
                .build();
    }

    // ------------------------------------------------------------ the player's voice

    /** Per run and player: the tone's phase, its ADPCM encoder, the packet count and the last time it was sent. */
    private static final Map<ScenarioRun, Map<String, double[]>> PHASE = java.util.Collections.synchronizedMap(new WeakHashMap<>());
    private static final Map<String, ImaAdpcm> ENCODERS = new java.util.concurrent.ConcurrentHashMap<>();

    static ServerPlayer speaker(ScenarioRun r) {
        return r.player().entity();
    }

    /**
     * The player speaks: the tone they "say" since the last call (wall-clock time, as a client's microphone
     * captures), sent through the server's voice packet handler as the client would.
     */
    static void talk(ScenarioRun r, ScenarioPlayer p, double hz, boolean talkKey) {
        talk(r, p, hz, talkKey, 0.5);
    }

    /** {@link #talk} at a given loudness (peak, 0..1 of full scale). */
    static void talk(ScenarioRun r, ScenarioPlayer p, double hz, boolean talkKey, double amplitude) {
        ServerPlayer sp = p.entity();
        double[] st = PHASE.computeIfAbsent(r, k -> new java.util.concurrent.ConcurrentHashMap<>())
                .computeIfAbsent(sp.getStringUUID(), k -> new double[] {0, 0, System.nanoTime()});
        long now = System.nanoTime();
        int n = (int) Math.min(VoicePackets.MAX_SAMPLES, (now - (long) st[2]) * VoicePackets.VOICE_RATE / 1_000_000_000L);
        if (n <= 0) return;
        st[2] = now;
        short[] pcm = new short[n];
        for (int i = 0; i < n; i++) {
            pcm[i] = (short) Math.round(Math.sin(st[0]) * amplitude * 32767);
            st[0] += 2 * Math.PI * hz / VoicePackets.VOICE_RATE;
        }
        ImaAdpcm enc = ENCODERS.computeIfAbsent(sp.getStringUUID(), k -> new ImaAdpcm());
        VoiceServer.onVoice(sp, new VoicePackets.Voice((int) st[1]++, VoicePackets.VOICE_RATE, n, enc.encode(pcm, 0, n), talkKey));
    }

    /** {@link #talk} every tick for {@code ticks} ticks; null when done. */
    static String talkFor(ScenarioRun r, ScenarioPlayer p, double hz, int ticks) {
        talk(r, p, hz, false);
        int[] c = COUNT.computeIfAbsent(r, k -> new int[1]);
        if (++c[0] < ticks) return "talking (" + c[0] + " ticks)";
        c[0] = 0;
        return null;
    }

    private static final Map<ScenarioRun, int[]> COUNT = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    /** The player walks to {@code rel} (standing on it, facing north). */
    static void stand(ScenarioRun r, ScenarioPlayer p, BlockPos rel) {
        BlockPos a = r.abs(rel);
        p.entity().moveTo(a.getX() + 0.5, a.getY(), a.getZ() + 0.5, 180, 0);
    }

    // ------------------------------------------------------------ radios

    /** Per run: the listener, and the listening tally (tone power, total power, samples, blocks). */
    private static final Map<ScenarioRun, ScenarioPlayer> LISTENERS = java.util.Collections.synchronizedMap(new WeakHashMap<>());
    private static final Map<ScenarioRun, double[]> TALLY = java.util.Collections.synchronizedMap(new WeakHashMap<>());
    private static final Map<ScenarioRun, Object> LAST_BLOCK = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static ScenarioPlayer listener(ScenarioRun r) {
        return LISTENERS.computeIfAbsent(r, k -> new ScenarioPlayer(r.level(), "Listener"));
    }

    static void handOutRadios(ScenarioRun r) {
        for (ScenarioPlayer p : List.of(r.player(), listener(r))) {
            stand(r, p, p == r.player() ? TALKER : LISTENER);
            holdRadio(r, p, RadioHandheldContent.WALKIE_TALKIE.get());
        }
    }

    /** The player takes a radio, right-clicks it on and tunes it to the channel. */
    static void holdRadio(ScenarioRun r, ScenarioPlayer p, net.minecraft.world.item.Item radio) {
        p.entity().setItemInHand(InteractionHand.MAIN_HAND, new ItemStack(radio));
        p.useHeld(false);   // right-click: on
        tune(r, p, FREQ_HZ);
        HandheldSettings s = HandheldSettings.read(p.entity().getMainHandItem());
        if (!s.on() || s.freqHz() != FREQ_HZ) r.fail("the radio isn't on and tuned: " + s);
    }

    static void tune(ScenarioRun r, ScenarioPlayer p, double hz) {
        // The tuning screen's packet: on, UHF, the frequency, volume 80, squelch off.
        RadioHandheldContent.applySettings(p.entity(), new HandheldPackets.Settings(true, true, HandheldBand.UHF.ordinal(), hz, 80, 0));
    }

    /** The tuning screen's PTT binding (its packet): Caps Lock, stored on the item. */
    static void bindPtt(ScenarioRun r) {
        RadioHandheldContent.applyPtt(speaker(r), new HandheldPackets.PttKey(true, "key.keyboard.caps.lock"));
        String k = com.example.evanscomputermod.radio.handheld.WalkieTalkieItem.ptt(speaker(r).getMainHandItem());
        if (!k.equals("key.keyboard.caps.lock")) r.fail("the PTT binding wasn't stored on the Walkie-Talkie: " + k);
    }

    private static final Map<ScenarioRun, Object> TALKER_AUDIO = java.util.Collections.synchronizedMap(new WeakHashMap<>());

    static void markTalkerAudio(ScenarioRun r) {
        HandheldServer.Session s = HandheldServer.sessionOf(speaker(r).getUUID());
        TALKER_AUDIO.put(r, s == null ? null : s.lastAudio);
    }

    /** Half duplex: while it was keyed, the talker's radio played no new audio. */
    static void checkHalfDuplex(ScenarioRun r) {
        HandheldServer.Session s = HandheldServer.sessionOf(speaker(r).getUUID());
        if (s == null) r.fail("the talker's radio has no session");
        else if (s.lastAudio != TALKER_AUDIO.get(r)) r.fail("the talker's radio played audio while it was transmitting");
        else if (!s.keyed()) r.fail("the talker's radio wasn't keyed");
    }

    /**
     * One tick of both radios (the talker holding Talk and saying {@code hz} when {@code talking}), adding what
     * the listener's radio played at {@code hz} to the tally; null after {@code ticks} ticks.
     */
    static String listen(ScenarioRun r, double hz, boolean talking, int ticks) {
        return listen(r, hz, talking, ticks, 0.5);
    }

    static String listen(ScenarioRun r, double hz, boolean talking, int ticks, double amplitude) {
        ScenarioPlayer talker = r.player(), lis = listener(r);
        if (talking) {
            VoiceServer.onTalk(talker.entity(), true);
            talk(r, talker, hz, true, amplitude);
        }
        HandheldServer.tickPlayer(talker.entity());
        HandheldServer.tickPlayer(lis.entity());
        double[] t = TALLY.computeIfAbsent(r, k -> new double[5]);
        t[4] = hz;
        HandheldServer.Session s = HandheldServer.sessionOf(lis.entity().getUUID());
        if (s != null && s.lastAudio != LAST_BLOCK.get(r)) {
            float[] a = s.lastAudio;
            LAST_BLOCK.put(r, a);
            t[0] += goertzel(a, t[4], HandheldServer.AUDIO_RATE) * a.length;
            for (float v : a) t[1] += v * v;
            t[2] += a.length;
            t[3]++;
        }
        int[] c = COUNT.computeIfAbsent(r, k -> new int[1]);
        if (++c[0] < ticks) return "listening (" + c[0] + " ticks)";
        c[0] = 0;
        return null;
    }

    /** Judge the tally: the listener heard the tone ({@code want}) or not; then clear it. */
    static void check(ScenarioRun r, String what, boolean want) {
        double[] t = TALLY.remove(r);
        HandheldServer.Session s = HandheldServer.sessionOf(listener(r).entity().getUUID());
        HandheldServer.Session tx = HandheldServer.sessionOf(r.player().entity().getUUID());
        if (t == null || t[2] == 0) {
            r.fail(what + ": the listener's radio played nothing at all");
            return;
        }
        double rms = Math.sqrt(t[1] / t[2]);
        double toneRms = Math.sqrt(2 * t[0] / t[2]);
        double share = t[1] > 0 ? t[0] * 2 / t[1] : 0;
        String line = String.format(Locale.ROOT, "%s: listener %.1f dBm, audio RMS %.3f, %.0f Hz tone RMS %.3f (%.0f%% of the audio), %d blocks; talker sent %d samples",
                what, s == null ? -200 : s.lastSignalDbm, rms, t[4], toneRms, share * 100, (long) t[3], tx == null ? 0 : tx.txSamples);
        EvansComputerMod.LOGGER.info("[radio_voice] {}", line);
        r.say("§7  " + line);
        if (want && !(toneRms > 0.05 && share > 0.3)) r.fail(what + ": the listener doesn't hear the tone: " + line);
        if (!want && toneRms > 0.05 && share > 0.3) r.fail(what + ": the listener hears the tone when it shouldn't: " + line);
    }

    static double goertzel(float[] x, double f, double fs) {
        double w = 2 * Math.PI * f / fs, re = 0, im = 0;
        for (int n = 0; n < x.length; n++) {
            re += x[n] * Math.cos(w * n);
            im -= x[n] * Math.sin(w * n);
        }
        return (re * re + im * im) / ((double) x.length * x.length);
    }

    // ------------------------------------------------------------ building

    private static void placeSpeakerAndMic(ScenarioRun run) {
        run.player().place(SpeakerContent.SPEAKER.get(), run.abs(SPEAKER), Direction.SOUTH,
                s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
        placeMic(run);
    }

    private static void placeMic(ScenarioRun run) {
        run.player().place(VoiceContent.MICROPHONE.get(), run.abs(MIC), Direction.SOUTH,
                s -> s.getValue(MicrophoneBlock.FACING) == Direction.SOUTH);
    }

    private static void buildStation(ScenarioRun r) {
        ScenarioPlayer p = r.player();
        p.place(RadioSdrContent.SDR_STANDARD.get(), r.abs(STATION_SDR), Direction.NORTH, s -> s.getValue(SdrBlock.FACING) == Direction.NORTH);
        p.place(VoiceContent.MICROPHONE.get(), r.abs(STATION_MIC), Direction.NORTH, s -> s.getValue(MicrophoneBlock.FACING) == Direction.NORTH);
        p.place(SpeakerContent.SPEAKER.get(), r.abs(STATION_SPEAKER), Direction.SOUTH, s -> s.getValue(SpeakerBlock.FACING) == Direction.SOUTH);
    }

    static boolean lit(ScenarioRun r) {
        var st = r.level().getBlockState(r.abs(MIC));
        return st.getBlock() instanceof MicrophoneBlock && st.getValue(MicrophoneBlock.LIVE)
                && r.level().getBlockEntity(r.abs(MIC)) instanceof MicrophoneBlockEntity;
    }

    /** {@link PlayerKit#typeFiles} with files bundled in the mod (computer path -> resource name). */
    private static void typeBundled(Scenario.Builder b, Map<String, String> files) {
        Map<String, String> texts = new LinkedHashMap<>();
        for (var e : new java.util.TreeMap<>(files).entrySet()) {
            String resource = "/evanscomputermod/scenarios/" + e.getValue();
            try (InputStream in = VoiceScenarios.class.getResourceAsStream(resource)) {
                if (in == null) throw new IllegalStateException("missing bundled file " + resource);
                String text = new String(in.readAllBytes(), StandardCharsets.UTF_8).replace("\r\n", "\n");
                texts.put(e.getKey(), text.endsWith("\n") ? text.substring(0, text.length() - 1) : text);
            } catch (IOException ex) {
                throw new IllegalStateException("reading " + resource, ex);
            }
        }
        PlayerKit.typeFiles(b, Map.of("pc", texts));
    }
}
//?}
