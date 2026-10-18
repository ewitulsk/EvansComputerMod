package com.example.evanscomputermod.speaker;

import com.example.evanscomputermod.voice.MicAudio;
import org.jetbrains.annotations.Nullable;

import java.io.IOException;
import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.ArrayList;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;
import java.util.TreeMap;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;
import java.util.function.Supplier;

/**
 * A computer's Volume Mixer (docs/gui/APPS.md "Volume Mixer"): one strip per app that plays
 * sound (its volume 0-150 %, mute, the speaker it plays on, a live peak meter and whether a
 * louder voice or phone stream ducks it), one master strip per speaker (volume 0-100, mute,
 * meter) and one strip per microphone (gain, level). It is the hardware side of the
 * {@code mixer} command, Settings &gt; Sound &gt; Mixer, the phone's volume slider and the
 * desktop's tray popup, which all read and write {@code /dev/mixer}.
 *
 * <p>An app is its app id; a program that isn't an app is its program name ({@code aplay}).
 * Settings are kept per app in {@code /var/lib/apps/mixer.conf}, the app database no app
 * may write, and apply to every stream the app opens, now and later.
 *
 * <pre>
 * /dev/mixer, read:
 *   output left volume=100 mute=0 peak=1200 label=Speaker
 *   app dev.ecm.music volume=80 mute=0 route=- on=left peak=5000 ducked=1 role=media streams=1
 *   input top gain=50 level=-42
 * /dev/mixer, write (one per line):
 *   set &lt;app&gt; &lt;0-150&gt;        set master[.&lt;side&gt;] &lt;0-100&gt;
 *   mute &lt;app|master[.&lt;side&gt;]&gt; [on|off|toggle]
 *   route &lt;app&gt; &lt;side|-&gt;        gain &lt;input side&gt; &lt;0-100&gt;
 * </pre>
 */
public final class VolumeMixer {

    /** Where the per-app settings live (under the computer's disk). */
    public static final String FILE = "var/lib/apps/mixer.conf";
    public static final int MAX_APP_PERCENT = 150;

    /** What the mixer needs from its computer. */
    public interface Host {
        /** Attached speakers by side ("left", "speaker" on a phone), in a stable order. */
        Map<String, SpeakerAudio> outputs();

        /** Attached microphones by side. */
        Map<String, MicAudio> inputs();

        /** The app id of a program, or its program name; null if it is gone. */
        @Nullable
        String keyOf(int pid);

        /** The computer's disk, or null (no persistence). */
        @Nullable
        Path storageRoot();

        /** A microphone's gain was changed here: let its owner save it. */
        default void inputGainChanged(String side) {}

        /** A speaker's master volume was changed here: let its owner save it. */
        default void outputVolumeChanged(String side) {}
    }

    /** One app's strip settings. */
    record Setting(int percent, boolean mute, @Nullable String route) {
        static final Setting DEFAULT = new Setting(100, false, null);
    }

    private final Host host;
    private final Map<String, Setting> settings = new ConcurrentHashMap<>();
    private volatile boolean loaded;

    public VolumeMixer(Host host) {
        this.host = host;
    }

    // ------------------------------------------------------------ streams

    /**
     * Where program {@code pid}'s default {@code /dev/audio} plays: the speaker its app is
     * routed to, else {@code fallback}. Asked before every write, so a route change moves
     * the stream.
     */
    public Supplier<SpeakerAudio> routeFor(int pid, SpeakerAudio fallback) {
        return () -> {
            String key = host.keyOf(pid);
            Setting s = key == null ? null : setting(key);
            if (s != null && s.route() != null) {
                SpeakerAudio c = host.outputs().get(s.route());
                if (c != null && !c.isClosed()) return c;
            }
            return fallback;
        };
    }

    /** A stream of program {@code pid} started: give it its app's volume and mute. */
    public Consumer<AudioStream> applier(int pid) {
        return s -> {
            String key = host.keyOf(pid);
            Setting st = key == null ? Setting.DEFAULT : setting(key);
            s.setMixer(st.percent(), st.mute());
        };
    }

    private Setting setting(String key) {
        load();
        return settings.getOrDefault(key, Setting.DEFAULT);
    }

    /** Push {@code key}'s settings to every open stream of the app. */
    private void apply(String key) {
        Setting st = setting(key);
        for (SpeakerAudio card : host.outputs().values()) {
            for (AudioStream s : card.streams()) {
                if (key.equals(host.keyOf(s.pid()))) s.setMixer(st.percent(), st.mute());
            }
        }
    }

    // ------------------------------------------------------------ /dev/mixer

    /** The strips, one per line (see the class comment). */
    public String text() {
        StringBuilder sb = new StringBuilder();
        Map<String, SpeakerAudio> outs = host.outputs();
        for (var e : outs.entrySet()) {
            SpeakerAudio c = e.getValue();
            sb.append("output ").append(e.getKey()).append(" volume=").append(c.volume()).append(" mute=").append(c.muted() ? 1 : 0)
                    .append(" peak=").append(c.peak()).append(" label=").append(c.label().replace(' ', '_')).append('\n');
        }
        // Apps: every app with a stream now, plus every app with saved settings.
        record App(String side, AudioStream s) {}
        Map<String, List<App>> apps = new TreeMap<>();
        for (var e : outs.entrySet()) {
            for (AudioStream s : e.getValue().streams()) {
                if (s.pid() == 0) continue;
                String key = host.keyOf(s.pid());
                if (key != null) apps.computeIfAbsent(key, k -> new ArrayList<>()).add(new App(e.getKey(), s));
            }
        }
        load();
        for (String k : settings.keySet()) apps.computeIfAbsent(k, x -> new ArrayList<>());
        for (var e : apps.entrySet()) {
            Setting st = setting(e.getKey());
            int peak = 0;
            boolean ducked = false;
            AudioStream.Role role = AudioStream.Role.MEDIA;
            String on = "-";
            int live = 0;
            for (App a : e.getValue()) {
                if (a.s().isClosed()) continue;
                live++;
                peak = Math.max(peak, a.s().peak());
                ducked |= a.s().ducked();
                if (a.s().role().ordinal() > role.ordinal()) role = a.s().role();
                if (on.equals("-")) on = a.side();
            }
            sb.append("app ").append(e.getKey()).append(" volume=").append(st.percent()).append(" mute=").append(st.mute() ? 1 : 0)
                    .append(" route=").append(st.route() == null ? "-" : st.route()).append(" on=").append(on)
                    .append(" peak=").append(peak).append(" ducked=").append(ducked ? 1 : 0).append(" role=").append(role.id())
                    .append(" streams=").append(live).append('\n');
        }
        for (var e : host.inputs().entrySet()) {
            MicAudio m = e.getValue();
            sb.append("input ").append(e.getKey()).append(" gain=").append(m.gain()).append(" level=").append(m.levelDb()).append('\n');
        }
        return sb.toString();
    }

    /**
     * One command (see the class comment).
     *
     * @throws IllegalArgumentException a bad command or value (EINVAL), with the reason
     */
    public void command(String line) {
        String[] p = line.trim().split("\\s+");
        if (p.length == 0 || p[0].isEmpty()) return;
        String cmd = p[0].toLowerCase(Locale.ROOT);
        switch (cmd) {
            case "set" -> {
                need(p, 3, "set <app|master[.side]> <percent>");
                int v = number(p[2]);
                SpeakerAudio master = master(p[1]);
                if (master != null) {
                    if (v < 0 || v > 100) throw new IllegalArgumentException("a speaker's volume is 0-100");
                    master.setVolume(v);
                    host.outputVolumeChanged(side(p[1]));
                    return;
                }
                if (v < 0 || v > MAX_APP_PERCENT) throw new IllegalArgumentException("an app's volume is 0-" + MAX_APP_PERCENT);
                update(p[1], s -> new Setting(v, s.mute(), s.route()));
            }
            case "mute", "unmute" -> {
                if (p.length < 2) throw new IllegalArgumentException("usage: mute <app|master[.side]> [on|off|toggle]");
                String how = cmd.equals("unmute") ? "off" : p.length > 2 ? p[2].toLowerCase(Locale.ROOT) : "on";
                SpeakerAudio master = master(p[1]);
                if (master != null) {
                    master.setMuted(flag(how, master.muted()));
                    return;
                }
                update(p[1], s -> new Setting(s.percent(), flag(how, s.mute()), s.route()));
            }
            case "route" -> {
                need(p, 3, "route <app> <side|->");
                String to = p[2].equals("-") ? null : p[2];
                if (to != null && !host.outputs().containsKey(to)) {
                    throw new IllegalArgumentException("no speaker '" + to + "' (speakers: " + String.join(" ", host.outputs().keySet()) + ")");
                }
                update(p[1], s -> new Setting(s.percent(), s.mute(), to));
            }
            case "gain" -> {
                need(p, 3, "gain <input side> <0-100>");
                MicAudio m = host.inputs().get(p[1]);
                if (m == null) throw new IllegalArgumentException("no microphone '" + p[1] + "'");
                int v = number(p[2]);
                if (v < 0 || v > 100) throw new IllegalArgumentException("gain is 0-100");
                m.setGain(v);
                host.inputGainChanged(p[1]);
            }
            case "reset" -> {
                need(p, 2, "reset <app>");
                load();
                settings.remove(p[1]);
                save();
                apply(p[1]);
            }
            default -> throw new IllegalArgumentException("unknown command '" + cmd + "' (set, mute, unmute, route, gain, reset)");
        }
    }

    private static void need(String[] p, int n, String usage) {
        if (p.length != n) throw new IllegalArgumentException("usage: " + usage);
    }

    private static int number(String s) {
        try {
            return Integer.parseInt(s.endsWith("%") ? s.substring(0, s.length() - 1) : s);
        } catch (NumberFormatException e) {
            throw new IllegalArgumentException("not a number: " + s);
        }
    }

    private static boolean flag(String how, boolean now) {
        return switch (how) {
            case "on", "1", "yes" -> true;
            case "off", "0", "no" -> false;
            case "toggle" -> !now;
            default -> throw new IllegalArgumentException("mute on, off or toggle");
        };
    }

    private static String side(String target) {
        int dot = target.indexOf('.');
        return dot < 0 ? "" : target.substring(dot + 1);
    }

    /** {@code master} (the first speaker) or {@code master.<side>}; null if {@code target} is an app. */
    @Nullable
    private SpeakerAudio master(String target) {
        if (!target.equals("master") && !target.startsWith("master.")) return null;
        Map<String, SpeakerAudio> outs = host.outputs();
        String side = side(target);
        SpeakerAudio c = side.isEmpty() ? outs.values().stream().findFirst().orElse(null) : outs.get(side);
        if (c == null) throw new IllegalArgumentException(outs.isEmpty() ? "no speaker attached" : "no speaker '" + side + "'");
        return c;
    }

    private void update(String key, java.util.function.UnaryOperator<Setting> f) {
        if (key.isEmpty() || key.length() > 128 || key.chars().anyMatch(Character::isWhitespace)) throw new IllegalArgumentException("bad app name");
        load();
        settings.put(key, f.apply(settings.getOrDefault(key, Setting.DEFAULT)));
        save();
        apply(key);
    }

    // ------------------------------------------------------------ persistence

    private synchronized void load() {
        if (loaded) return;
        loaded = true;
        Path root = host.storageRoot();
        if (root == null) return;
        try {
            Path f = root.resolve(FILE);
            if (!Files.isRegularFile(f)) return;
            for (String l : Files.readAllLines(f, StandardCharsets.UTF_8)) {
                String[] p = l.trim().split("\\s+");
                if (p.length < 3 || p[0].startsWith("#")) continue;
                try {
                    int v = Math.max(0, Math.min(MAX_APP_PERCENT, Integer.parseInt(p[1])));
                    settings.put(p[0], new Setting(v, p[2].equals("1"), p.length > 3 && !p[3].equals("-") ? p[3] : null));
                } catch (NumberFormatException ignored) {
                    // a damaged line: skip it
                }
            }
        } catch (IOException ignored) {
            // unreadable: defaults
        }
    }

    private synchronized void save() {
        Path root = host.storageRoot();
        if (root == null) return;
        StringBuilder sb = new StringBuilder("# Volume Mixer: <app> <volume 0-150> <mute 0|1> <speaker|->\n");
        for (var e : new TreeMap<>(settings).entrySet()) {
            Setting s = e.getValue();
            sb.append(e.getKey()).append(' ').append(s.percent()).append(' ').append(s.mute() ? 1 : 0).append(' ')
                    .append(s.route() == null ? "-" : s.route()).append('\n');
        }
        try {
            Path f = root.resolve(FILE);
            Files.createDirectories(f.getParent());
            Files.writeString(f, sb.toString(), StandardCharsets.UTF_8);
        } catch (IOException ignored) {
            // a full disk only costs persistence
        }
    }

    // ------------------------------------------------------------ the device

    /**
     * {@code /dev/mixer}: read the strips (a fresh snapshot after EOF), write commands.
     * Writing needs the {@code mixer} permission for apps.
     */
    public static final class Fd implements com.example.evanscomputermod.computer.wasi.DeviceFd {
        private final VolumeMixer mixer;
        private final java.util.function.BooleanSupplier mayWrite;
        private Boolean writable;
        private byte[] snapshot;
        private int pos;
        private final StringBuilder partial = new StringBuilder();

        /** {@code mayWrite} is asked at the first write (it may ask the player). */
        public Fd(VolumeMixer mixer, java.util.function.BooleanSupplier mayWrite) {
            this.mixer = mixer;
            this.mayWrite = mayWrite;
        }

        @Override
        public synchronized int read(byte[] buf, int offset, int length) {
            if (snapshot == null) {
                snapshot = mixer.text().getBytes(StandardCharsets.UTF_8);
                pos = 0;
            }
            int n = Math.min(length, snapshot.length - pos);
            System.arraycopy(snapshot, pos, buf, offset, n);
            pos += n;
            if (n == 0) snapshot = null;
            return n;
        }

        @Override
        public synchronized int write(byte[] data, int offset, int length) throws IOException {
            if (writable == null) writable = mayWrite.getAsBoolean();
            if (!writable) throw new ErrnoException(63, "changing the Volume Mixer needs the mixer permission");
            partial.append(new String(data, offset, length, StandardCharsets.UTF_8));
            int nl;
            while ((nl = partial.indexOf("\n")) >= 0) {
                String line = partial.substring(0, nl);
                partial.delete(0, nl + 1);
                apply(line);
            }
            if (partial.length() > 0 && partial.length() < 512) {
                String line = partial.toString();
                partial.setLength(0);
                apply(line);
            }
            snapshot = null;
            return length;
        }

        private void apply(String line) throws IOException {
            try {
                mixer.command(line);
            } catch (IllegalArgumentException e) {
                throw new ErrnoException(ErrnoException.EINVAL, e.getMessage());
            }
        }

        @Override public void close() {}
        @Override public boolean isReadable() { return true; }
        @Override public boolean isWritable() { return true; }
        @Override public boolean isNonBlocking() { return false; }
        @Override public void setNonBlocking(boolean nb) {}
    }

    /** Settings by app (tests). */
    Map<String, Setting> settings() {
        load();
        return new LinkedHashMap<>(settings);
    }
}
