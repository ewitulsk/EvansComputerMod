package com.example.evanscomputermod.testing.scenario;

import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;

import java.util.ArrayList;
import java.util.HashMap;
import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;
import java.util.Set;
import java.util.regex.Pattern;

/**
 * A debug scenario: a layout of terminals and cables (in coordinates
 * relative to an origin) plus a script of terminal steps. The same
 * definition is spawned into the world by {@code /ecm scenario spawn} and
 * run headless as a GameTest (MC 26.1), so what you watch in game is what
 * CI checks.
 *
 * <p>Terminals are placed with their screen facing {@link Node#facing}. A
 * terminal's NICs are its non-screen faces in {@link Direction} order
 * (DOWN, UP, NORTH, SOUTH, WEST, EAST), so for a north-facing terminal
 * eth0 = down, eth1 = up, eth2 = south, eth3 = west, eth4 = east;
 * {@link #eth} computes it. Each cable run is its own segment, so
 * {@link Builder#build} rejects layouts where runs touch or a run touches
 * a terminal face it wasn't meant to.
 */
public final class Scenario {
    public enum Role { HOST, SWITCH }

    public record Node(String name, BlockPos pos, Direction facing, Role role, String ip) {}

    /** A cable run from {@code a}'s face {@code faceA} to {@code b}'s face {@code faceB}. */
    public record Link(String name, String a, Direction faceA, String b, Direction faceB, List<BlockPos> cable) {}

    public sealed interface Step permits Send, Expect, Until, Wait, Cut, Note, Mutation, Await {}
    /**
     * Poll {@code check} every tick: null means satisfied; any other string is the current
     * state, and the scenario fails with it if {@code timeoutMs} passes first.
     */
    public record Await(java.util.function.Function<ScenarioRun, String> check, String what, int timeoutMs) implements Step {}
    public record Mutation(java.util.function.Consumer<ScenarioRun> apply,String what) implements Step {}
    /** Type a line on a terminal. */
    public record Send(String node, String line) implements Step {}
    /**
     * Wait until {@code re} matches the output of the last line sent to
     * {@code node}; if {@code failRe} (may be null) matches first, fail at once.
     */
    public record Expect(String node, Pattern re, String what, Pattern failRe) implements Step {}
    /** Re-send {@code line} every {@code everyMs} until its output matches {@code re} (convergence). */
    public record Until(String node, String line, Pattern re, String what, int everyMs) implements Step {}
    public record Wait(int ms, String why) implements Step {}
    /** Break a cable: remove block {@code index} of link {@code link}. */
    public record Cut(String link, int index, String what) implements Step {}
    /** A heading shown in chat (and in the printed walkthrough). */
    public record Note(String text) implements Step {}

    /**
     * Anything else a scenario places (floors, walls, sensors, modules,
     * files on a computer). Built after the terminals and cables, removed
     * before the box is cleared.
     */
    public interface Decor {
        /** Blocks it occupies, relative to the origin (they are cleared on build and clear). */
        List<BlockPos> footprint();

        void build(ScenarioRun run);

        /** Remove what clearing the box wouldn't (entities, say). */
        default void clear(ScenarioRun run) {
        }
    }

    public final String name;
    public final String description;
    public final Map<String, Node> nodes;
    public final List<Link> links;
    public final List<Step> steps;
    public final List<Decor> decor;
    /** Wall-clock limit for the whole run. */
    public final long timeLimitMs;

    private Scenario(String name, String description, Map<String, Node> nodes, List<Link> links,
                     List<Step> steps, List<Decor> decor, long timeLimitMs) {
        this.name = name;
        this.description = description;
        this.nodes = nodes;
        this.links = links;
        this.steps = steps;
        this.decor = decor;
        this.timeLimitMs = timeLimitMs;
    }

    /** NIC index of a face on a terminal with the given screen direction (no Interface blocks). */
    public static int eth(Direction facing, Direction face) {
        if (face == facing) throw new IllegalArgumentException("the screen face has no NIC");
        int i = 0;
        for (Direction d : Direction.values()) {
            if (d == face) return i;
            if (d != facing) i++;
        }
        throw new AssertionError();
    }

    public String eth(String node, Direction face) {
        return "eth" + eth(nodes.get(node).facing(), face);
    }

    /** Every block the layout occupies, plus screen fronts (kept clear so the screens can be seen). */
    public List<BlockPos> footprint() {
        List<BlockPos> all = new ArrayList<>();
        for (Node n : nodes.values()) {
            all.add(n.pos());
            all.add(n.pos().relative(n.facing()));
        }
        for (Link l : links) all.addAll(l.cable());
        for (Decor d : decor) all.addAll(d.footprint());
        return all;
    }

    public BlockPos min() {
        List<BlockPos> f = footprint();
        int x = Integer.MAX_VALUE, y = Integer.MAX_VALUE, z = Integer.MAX_VALUE;
        for (BlockPos p : f) { x = Math.min(x, p.getX()); y = Math.min(y, p.getY()); z = Math.min(z, p.getZ()); }
        return new BlockPos(x, y, z);
    }

    public BlockPos max() {
        List<BlockPos> f = footprint();
        int x = Integer.MIN_VALUE, y = Integer.MIN_VALUE, z = Integer.MIN_VALUE;
        for (BlockPos p : f) { x = Math.max(x, p.getX()); y = Math.max(y, p.getY()); z = Math.max(z, p.getZ()); }
        return new BlockPos(x, y, z);
    }

    /** The script as text: what to type on which terminal, for doing it by hand. */
    public List<String> walkthrough() {
        List<String> out = new ArrayList<>();
        String last = null;
        for (Step s : steps) {
            switch (s) {
                case Note n -> { out.add("-- " + n.text()); last = null; }
                case Mutation m -> out.add("-- " + m.what());
                case Await a -> { out.add("-- wait (up to " + (a.timeoutMs() / 1000) + " s) until " + a.what()); last = null; }
                case Send c -> {
                    if (!c.node().equals(last)) out.add("[" + c.node() + "]");
                    out.add("  " + c.line());
                    last = c.node();
                }
                case Until u -> {
                    if (!u.node().equals(last)) out.add("[" + u.node() + "]");
                    out.add("  " + u.line() + "   (repeat until: " + u.what() + ")");
                    last = u.node();
                }
                case Expect e -> out.add("      => " + e.what());
                case Wait w -> { out.add("-- wait " + (w.ms() / 1000) + " s: " + w.why()); last = null; }
                case Cut c -> { out.add("-- break a cable of " + c.link() + ": " + c.what()); last = null; }
            }
        }
        return out;
    }

    public static Builder builder(String name, String description) {
        return new Builder(name, description);
    }

    /** A straight-segment cable path: {@code Path.from(p).go(EAST, 2).go(DOWN, 2)}. */
    public static final class Path {
        private final List<BlockPos> blocks = new ArrayList<>();

        public static Path from(BlockPos start) {
            Path p = new Path();
            p.blocks.add(start);
            return p;
        }

        public Path go(Direction d, int n) {
            for (int i = 0; i < n; i++) blocks.add(blocks.get(blocks.size() - 1).relative(d));
            return this;
        }

        public List<BlockPos> blocks() {
            return List.copyOf(blocks);
        }
    }

    public static final class Builder {
        private final String name;
        private final String description;
        private final Map<String, Node> nodes = new LinkedHashMap<>();
        private final List<Link> links = new ArrayList<>();
        private final List<Step> steps = new ArrayList<>();
        private final List<Decor> decor = new ArrayList<>();
        private long timeLimitMs = 60_000;

        private Builder(String name, String description) {
            this.name = name;
            this.description = description;
        }

        public Builder host(String name, BlockPos pos, String ip) {
            nodes.put(name, new Node(name, pos, Direction.NORTH, Role.HOST, ip));
            return this;
        }

        public Builder switchNode(String name, BlockPos pos) {
            nodes.put(name, new Node(name, pos, Direction.NORTH, Role.SWITCH, null));
            return this;
        }

        /** Cable from {@code a}'s face to {@code b}'s face; the path must start and end next to those faces. */
        public Builder link(String name, String a, Direction faceA, String b, Direction faceB, Path path) {
            links.add(new Link(name, a, faceA, b, faceB, path.blocks()));
            return this;
        }

        public Builder decor(Decor d) {
            decor.add(d);
            return this;
        }

        public Builder timeLimit(long ms) {
            timeLimitMs = ms;
            return this;
        }

        /** NIC name of {@code node}'s {@code face} (the node must be declared already). */
        public String eth(String node, Direction face) {
            return "eth" + Scenario.eth(nodes.get(node).facing(), face);
        }

        public Builder note(String text) { steps.add(new Note(text)); return this; }

        public Builder send(String node, String... lines) {
            for (String l : lines) steps.add(new Send(node, l));
            return this;
        }

        /** {@code re} is matched per line (MULTILINE). */
        public Builder expect(String node, String re, String what) {
            steps.add(new Expect(node, Pattern.compile(re, Pattern.MULTILINE), what, null));
            return this;
        }

        public Builder until(String node, String line, String re, String what) {
            steps.add(new Until(node, line, Pattern.compile(re, Pattern.MULTILINE), what, 2_000));
            return this;
        }

        public Builder waitMs(int ms, String why) { steps.add(new Wait(ms, why)); return this; }
        public Builder mutate(java.util.function.Consumer<ScenarioRun> action,String what) {steps.add(new Mutation(action,what));return this;}

        /** Wait until {@code check} returns null (polled every tick), failing with its last answer after {@code timeoutMs}. */
        public Builder await(java.util.function.Function<ScenarioRun, String> check, String what, int timeoutMs) {
            steps.add(new Await(check, what, timeoutMs));
            return this;
        }

        public Builder cut(String link, int index, String what) { steps.add(new Cut(link, index, what)); return this; }

        /** Send a ping and wait for its summary line; a different reply count fails at once. */
        public Builder ping(String from, String ip, int count, int received, String what) {
            send(from, "ping " + ip + " -n " + count);
            steps.add(new Expect(from, Pattern.compile("^" + count + " packets sent, " + received + " received", Pattern.MULTILINE),
                    what, Pattern.compile("^" + count + " packets sent, \\d+ received", Pattern.MULTILINE)));
            return this;
        }

        /** Give every host its address (ifconfig eth0). */
        public Builder configureHosts() {
            note("Hosts: address eth0");
            for (Node n : nodes.values()) {
                if (n.role() != Role.HOST) continue;
                send(n.name(), "ifconfig eth0 " + n.ip());
                expect(n.name(), "eth0: inet " + Pattern.quote(n.ip()), n.name() + " has " + n.ip());
            }
            return this;
        }

        public Scenario build() {
            validate();
            return new Scenario(name, description, Map.copyOf(nodes), List.copyOf(links), List.copyOf(steps), List.copyOf(decor), timeLimitMs);
        }

        /** Each run must be a chain touching only its own two terminal faces and no other run. */
        private void validate() {
            Map<BlockPos, Link> owner = new HashMap<>();
            Map<BlockPos, Node> terminals = new HashMap<>();
            for (Node n : nodes.values()) terminals.put(n.pos(), n);
            for (Link l : links) {
                for (BlockPos p : l.cable()) {
                    if (owner.put(p, l) != null || terminals.containsKey(p)) {
                        throw new IllegalStateException(name + ": block " + p + " used twice");
                    }
                }
            }
            for (Link l : links) {
                Node a = nodes.get(l.a()), b = nodes.get(l.b());
                if (a == null || b == null) throw new IllegalStateException(name + ": " + l.name() + " names an unknown node");
                List<BlockPos> c = l.cable();
                if (!c.get(0).equals(a.pos().relative(l.faceA())) || !c.get(c.size() - 1).equals(b.pos().relative(l.faceB()))) {
                    throw new IllegalStateException(name + ": " + l.name() + " does not start/end at its faces");
                }
                Set<String> allowedFaces = new HashSet<>(List.of(a.name() + ":" + l.faceA(), b.name() + ":" + l.faceB()));
                for (BlockPos p : c) {
                    for (Direction d : Direction.values()) {
                        BlockPos q = p.relative(d);
                        Link other = owner.get(q);
                        if (other != null && other != l) {
                            throw new IllegalStateException(name + ": " + l.name() + " touches " + other.name() + " at " + q);
                        }
                        Node t = terminals.get(q);
                        if (t != null && !allowedFaces.contains(t.name() + ":" + d.getOpposite())) {
                            throw new IllegalStateException(name + ": " + l.name() + " touches " + t.name()
                                    + "'s " + d.getOpposite() + " face");
                        }
                    }
                }
            }
            for (Step s : steps) {
                String node = switch (s) {
                    case Send x -> x.node();
                    case Expect x -> x.node();
                    case Until x -> x.node();
                    default -> null;
                };
                if (node != null && !nodes.containsKey(node)) throw new IllegalStateException(name + ": unknown node " + node);
                if (s instanceof Cut cut && links.stream().noneMatch(l -> l.name().equals(cut.link()))) {
                    throw new IllegalStateException(name + ": unknown link " + cut.link());
                }
            }
        }
    }
}
