package com.example.evanscomputermod.testing.scenario;

import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.NetworkCableBlock;
import com.example.evanscomputermod.block.TerminalBlock;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import com.example.evanscomputermod.testing.scenario.Scenario.Cut;
import com.example.evanscomputermod.testing.scenario.Scenario.Expect;
import com.example.evanscomputermod.testing.scenario.Scenario.Link;
import com.example.evanscomputermod.testing.scenario.Scenario.Node;
import com.example.evanscomputermod.testing.scenario.Scenario.Note;
import com.example.evanscomputermod.testing.scenario.Scenario.Send;
import com.example.evanscomputermod.testing.scenario.Scenario.Step;
import com.example.evanscomputermod.testing.scenario.Scenario.Until;
import com.example.evanscomputermod.testing.scenario.Scenario.Wait;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.block.Blocks;
import net.minecraft.world.level.block.state.BlockState;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.List;
import java.util.Map;
import java.util.function.Consumer;
import java.util.regex.Pattern;

/**
 * One run of a {@link Scenario} in a world: {@link #build} places the
 * blocks at {@code origin}, then {@link #tick} (once per server tick) boots
 * the terminals and plays the script.
 *
 * <p>Output is read off the terminal screens. An {@link Expect} only looks
 * at what the terminal printed after the last line sent to it: the rows
 * below that line's echo, and only once the screen has changed since it was
 * sent (so output of an earlier, identical command can't satisfy it).
 */
public final class ScenarioRun {
    public enum State { RUNNING, PASSED, FAILED }

    private final Scenario sc;
    private final ServerLevel level;
    private final BlockPos origin;
    private final Consumer<String> log;
    /** Minimum ticks between two typed lines (so people can follow along). */
    private final int paceTicks;
    /** When false, the blocks are built and booted but no script runs. */
    private final boolean scripted;

    private State state = State.RUNNING;
    private String failure;
    private int index = 0;
    private boolean booted;
    private long startMs = -1;
    private long stepStartMs;
    private int ticksSinceSend = Integer.MAX_VALUE / 2;
    private final Map<String, Sent> sent = new HashMap<>();
    private long untilSentAt;
    private int untilIndex = -1;

    private static final class Sent {
        final String line;
        final String before;
        boolean changed;

        Sent(String line, String before) {
            this.line = line;
            this.before = before;
        }
    }

    public ScenarioRun(Scenario sc, ServerLevel level, BlockPos origin, Consumer<String> log,
                       int paceTicks, boolean scripted) {
        this.sc = sc;
        this.level = level;
        this.origin = origin;
        this.log = log;
        this.paceTicks = paceTicks;
        this.scripted = scripted;
    }

    public Scenario scenario() { return sc; }
    public BlockPos origin() { return origin; }
    public State state() { return state; }
    public String failure() { return failure; }

    public BlockPos abs(BlockPos rel) {
        return origin.offset(rel);
    }

    /** Clear the footprint's bounding box (plus a margin of 1) and place terminals and cables. */
    public void build() {
        BlockPos lo = abs(sc.min()).offset(-1, 0, -1), hi = abs(sc.max()).offset(1, 1, 1);
        clearBox(level, lo, hi);
        for (Node n : sc.nodes.values()) {
            level.setBlock(abs(n.pos()), ModBlocks.TERMINAL_BLOCK.get().defaultBlockState()
                    .setValue(TerminalBlock.FACING, n.facing()), 3);
        }
        for (Link l : sc.links) {
            for (BlockPos p : l.cable()) level.setBlock(abs(p), ModBlocks.NETWORK_CABLE.get().defaultBlockState(), 3);
        }
        // setBlock doesn't run getStateForPlacement: connect the arms ourselves.
        for (Link l : sc.links) {
            for (BlockPos rel : l.cable()) {
                BlockPos p = abs(rel);
                BlockState s = level.getBlockState(p);
                for (Direction d : Direction.values()) {
                    BlockPos q = p.relative(d);
                    s = s.setValue(NetworkCableBlock.getPropertyForDirection(d),
                            NetworkCableBlock.canConnectToFace(level.getBlockState(q), d.getOpposite(), level, q));
                }
                level.setBlock(p, s, 3);
            }
        }
    }

    /** Remove everything {@link #build} placed. */
    public void clear() {
        clearBox(level, abs(sc.min()).offset(-1, 0, -1), abs(sc.max()).offset(1, 1, 1));
    }

    private static void clearBox(ServerLevel level, BlockPos lo, BlockPos hi) {
        for (BlockPos p : BlockPos.betweenClosed(lo, hi)) {
            if (!level.getBlockState(p).isAir()) level.setBlock(p, Blocks.AIR.defaultBlockState(), 3);
        }
    }

    public void fail(String why) {
        if (state != State.RUNNING) return;
        state = State.FAILED;
        failure = why;
        log.accept("§cFAIL " + sc.name + ": " + why);
    }

    // ------------------------------------------------------------ running

    public State tick() {
        if (state != State.RUNNING) return state;
        long now = System.currentTimeMillis();
        if (startMs < 0) {
            startMs = now;
            stepStartMs = now;
        }
        ticksSinceSend++;
        try {
            if (!booted) {
                if (!boot()) {
                    checkTime(now, "booting the terminals");
                    return state;
                }
                booted = true;
                log.accept("§7booted " + sc.nodes.size() + " terminals in " + (now - startMs) / 1000.0 + " s");
                if (!scripted) {
                    state = State.PASSED;
                    return state;
                }
            }
            while (index < sc.steps.size() && state == State.RUNNING) {
                Step s = sc.steps.get(index);
                if (!run(s, now)) break;
                index++;
                stepStartMs = now;
            }
            if (state == State.RUNNING && index >= sc.steps.size()) {
                state = State.PASSED;
                log.accept("§aPASS " + sc.name + " in " + (now - startMs) / 1000.0 + " s");
            } else {
                checkTime(now, index < sc.steps.size() ? describe(sc.steps.get(index)) : "?");
            }
        } catch (RuntimeException e) {
            fail("step " + (index + 1) + " threw " + e);
        }
        return state;
    }

    private void checkTime(long now, String at) {
        if (state == State.RUNNING && now - startMs > sc.timeLimitMs) {
            fail("timed out after " + sc.timeLimitMs / 1000 + " s at step " + (index + 1) + "/"
                    + sc.steps.size() + ": " + at);
        }
    }

    private boolean boot() {
        boolean all = true;
        for (Node n : sc.nodes.values()) {
            TerminalBlockEntity be = be(n.name());
            if (be == null) throw new IllegalStateException("no terminal at " + abs(n.pos()) + " (" + n.name() + ")");
            be.initializeWasm(); // no-op once started
            all &= screen(n.name()).contains("Welcome to Terminal OS");
        }
        return all;
    }

    private boolean run(Step step, long now) {
        switch (step) {
            case Note n -> {
                log.accept("§e== " + n.text());
                return true;
            }
            case Send s -> {
                if (ticksSinceSend < paceTicks) return false;
                type(s.node(), s.line());
                return true;
            }
            case Expect e -> {
                String r = output(e.node());
                if (r == null) return false;
                if (!e.re().matcher(r).find()) {
                    java.util.regex.Matcher m = e.failRe() == null ? null : e.failRe().matcher(r);
                    if (m != null && m.find()) {
                        int eol = r.indexOf('\n', m.start());
                        fail("step " + (index + 1) + ": expected " + e.what() + ", got: "
                                + r.substring(m.start(), eol < 0 ? r.length() : eol).strip());
                    }
                    return false;
                }
                log.accept("§a  ok §f" + e.what());
                return true;
            }
            case Until u -> {
                if (untilIndex != index || now - untilSentAt >= u.everyMs()) {
                    if (ticksSinceSend < paceTicks) return false;
                    type(u.node(), u.line());
                    untilSentAt = now;
                    untilIndex = index;
                    return false;
                }
                String r = output(u.node());
                if (r == null || !u.re().matcher(r).find()) return false;
                log.accept("§a  ok §f" + u.what() + " §7(" + (now - stepStartMs) / 1000.0 + " s)");
                return true;
            }
            case Wait w -> {
                return now - stepStartMs >= w.ms();
            }
            case Cut c -> {
                Link l = sc.links.stream().filter(x -> x.name().equals(c.link())).findFirst().orElseThrow();
                BlockPos p = abs(l.cable().get(c.index()));
                level.setBlock(p, Blocks.AIR.defaultBlockState(), 3);
                log.accept("§6  cut §f" + c.link() + " at " + p.toShortString() + " §7(" + c.what() + ")");
                return true;
            }
        }
    }

    private void type(String node, String line) {
        sent.put(node, new Sent(line, screen(node)));
        be(node).onStringInput(line + "\n");
        ticksSinceSend = 0;
        if (paceTicks > 0) log.accept("§b  [" + node + "] §f" + line);
    }

    /**
     * What {@code node} printed after the last line sent to it (one screen
     * snapshot), or null while the screen hasn't changed since the send.
     */
    private String output(String node) {
        String scr = screen(node);
        Sent s = sent.get(node);
        if (s == null) return scr;
        if (!s.changed) {
            if (scr.equals(s.before)) return null;
            s.changed = true;
        }
        return after(scr, s.line);
    }

    /** Rows below the last row ending with {@code line} (its echo), or the whole screen if it scrolled off. */
    static String after(String screen, String line) {
        String[] rows = screen.split("\n", -1);
        for (int i = rows.length - 1; i >= 0; i--) {
            if (rows[i].stripTrailing().endsWith(line)) {
                StringBuilder sb = new StringBuilder();
                for (int j = i + 1; j < rows.length; j++) sb.append(rows[j].stripTrailing()).append('\n');
                return sb.toString();
            }
        }
        return screen;
    }

    private String describe(Step s) {
        return switch (s) {
            case Send x -> "send '" + x.line() + "' to " + x.node();
            case Expect x -> "wait for " + x.what() + " on " + x.node();
            case Until x -> "repeat '" + x.line() + "' on " + x.node() + " until " + x.what();
            case Wait x -> "wait " + x.ms() + " ms (" + x.why() + ")";
            case Cut x -> "cut " + x.link();
            case Note x -> x.text();
        };
    }

    // ------------------------------------------------------------ screens

    private TerminalBlockEntity be(String node) {
        return level.getBlockEntity(abs(sc.nodes.get(node).pos())) instanceof TerminalBlockEntity t ? t : null;
    }

    public String screen(String node) {
        TerminalBlockEntity be = be(node);
        return be == null ? "" : screen(be.getDisplay());
    }

    /** Screen text of a terminal (rows joined by newlines, trailing spaces kept). */
    public static String screen(TerminalDisplay d) {
        StringBuilder sb = new StringBuilder();
        for (int y = 0; y < d.getHeight(); y++) {
            for (int x = 0; x < d.getWidth(); x++) {
                int c = d.getCharAt(x, y) & 0xFF;
                sb.append(c >= 32 && c < 127 ? (char) c : ' ');
            }
            sb.append('\n');
        }
        return sb.toString();
    }

    /** All screens, for failure reports. */
    public String dump() {
        List<String> parts = new ArrayList<>();
        for (Node n : sc.nodes.values()) {
            parts.add("--- " + n.name() + " ---\n" + screen(n.name()).stripTrailing());
        }
        return String.join("\n", parts);
    }
}
