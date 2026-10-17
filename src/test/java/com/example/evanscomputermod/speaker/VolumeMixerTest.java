package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.voice.MicAudio;
import org.junit.jupiter.api.Test;
import org.junit.jupiter.api.io.TempDir;

import java.nio.file.Files;
import java.nio.file.Path;
import java.util.Map;
import java.util.TreeMap;

import static org.junit.jupiter.api.Assertions.*;

/** The Volume Mixer: per-app strips, master strips, routes, persistence and /dev/mixer's text. */
class VolumeMixerTest {

    @TempDir
    Path disk;

    private final SpeakerAudio left = new SpeakerAudio(), right = new SpeakerAudio();
    private final MicAudio mic = new MicAudio((start, n, rate, out) -> {}, () -> 0L, 1);
    private final Map<Integer, String> apps = new TreeMap<>(Map.of(10, "dev.ecm.music", 11, "dev.ecm.discord", 12, "aplay"));

    private VolumeMixer mixer() {
        return new VolumeMixer(new VolumeMixer.Host() {
            @Override public Map<String, SpeakerAudio> outputs() { return new TreeMap<>(Map.of("left", left, "right", right)); }
            @Override public Map<String, MicAudio> inputs() { return Map.of("top", mic); }
            @Override public String keyOf(int pid) { return apps.get(pid); }
            @Override public Path storageRoot() { return disk; }
        });
    }

    private static byte[] samples(int n, int v) {
        byte[] b = new byte[n * 2];
        for (int i = 0; i < n; i++) {
            b[2 * i] = (byte) v;
            b[2 * i + 1] = (byte) (v >> 8);
        }
        return b;
    }

    private AudioDeviceFd open(VolumeMixer m, int pid) {
        return new AudioDeviceFd(m.routeFor(pid, left), pid, m.applier(pid));
    }

    @Test
    void appVolumeAppliesToOpenAndLaterStreamsAndIsRemembered() throws Exception {
        VolumeMixer m = mixer();
        AudioDeviceFd music = open(m, 10);
        music.write(samples(10, 1000), 0, 20);
        m.command("set dev.ecm.music 40");
        assertEquals(0.4, music.stream().appGain, 1e-9, "an open stream follows at once");
        AudioDeviceFd again = open(m, 10);
        again.write(samples(10, 1000), 0, 20);
        assertEquals(0.4, again.stream().appGain, 1e-9, "a stream opened later starts at the app's volume");
        String saved = Files.readString(disk.resolve(VolumeMixer.FILE));
        assertTrue(saved.contains("dev.ecm.music 40 0 -"), saved);
        // A new mixer (the computer rebooted) reads it back.
        AudioDeviceFd later = open(mixer(), 10);
        later.write(samples(10, 1000), 0, 20);
        assertEquals(0.4, later.stream().appGain, 1e-9);
    }

    @Test
    void muteRouteAndMasterCommands() throws Exception {
        VolumeMixer m = mixer();
        AudioDeviceFd discord = open(m, 11);
        discord.write(samples(10, 1000), 0, 20);
        m.command("mute dev.ecm.discord");
        assertTrue(discord.stream().muted);
        m.command("mute dev.ecm.discord toggle");
        assertFalse(discord.stream().muted);
        m.command("route dev.ecm.discord right");
        discord.write(samples(10, 1000), 0, 20);
        assertSame(right, discord.stream().card, "routed to the right speaker");
        m.command("set master.right 30");
        assertEquals(30, right.volume());
        m.command("mute master");
        assertTrue(left.muted(), "master alone is the first speaker");
        m.command("gain top 70");
        assertEquals(70, mic.gain());
        assertThrows(IllegalArgumentException.class, () -> m.command("set dev.ecm.music 151"));
        assertThrows(IllegalArgumentException.class, () -> m.command("set master 101"));
        assertThrows(IllegalArgumentException.class, () -> m.command("route aplay nowhere"));
        assertThrows(IllegalArgumentException.class, () -> m.command("bogus"));
    }

    @Test
    void textListsSpeakersAppsAndMicrophones() throws Exception {
        VolumeMixer m = mixer();
        AudioDeviceFd aplay = open(m, 12);
        aplay.write(samples(10, 1000), 0, 20);
        m.command("set dev.ecm.music 80");
        String t = m.text();
        assertTrue(t.contains("output left volume=100 mute=0"), t);
        assertTrue(t.contains("output right volume=100"), t);
        assertTrue(t.contains("app aplay volume=100 mute=0 route=- on=left"), t);
        assertTrue(t.contains("app dev.ecm.music volume=80 mute=0 route=- on=- peak=0 ducked=0 role=media streams=0"), t);
        assertTrue(t.contains("input top gain=50 level="), t);
    }

    @Test
    void theDeviceRefusesWritesWithoutPermission() throws Exception {
        VolumeMixer m = mixer();
        VolumeMixer.Fd denied = new VolumeMixer.Fd(m, () -> false);
        byte[] cmd = "set aplay 10\n".getBytes();
        var e = assertThrows(com.example.evanscomputermod.computer.wasi.DeviceFd.ErrnoException.class, () -> denied.write(cmd, 0, cmd.length));
        assertEquals(63, e.errno());
        VolumeMixer.Fd ok = new VolumeMixer.Fd(m, () -> true);
        ok.write(cmd, 0, cmd.length);
        byte[] buf = new byte[4096];
        int n = ok.read(buf, 0, buf.length);
        assertTrue(new String(buf, 0, n).contains("app aplay volume=10"));
    }
}
