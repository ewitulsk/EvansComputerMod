package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.testing.ClientMirror;
import com.example.evanscomputermod.testing.ScreenCapture;
import com.example.evanscomputermod.testing.scenario.Scenario;
import com.example.evanscomputermod.testing.scenario.ScenarioRun;
import com.example.evanscomputermod.testing.scenario.SwitchScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

import java.util.List;
import java.util.regex.Pattern;

/**
 * Client screen sync, namespace {@code ecm_sync}: does what a client shows
 * match the server's screen?
 *
 * <p>Reproduces the reported bug: run {@code switch_vlans}, open sw1's
 * terminal, type {@code vlans show}; the client showed the output twice
 * until the GUI was reopened. Each case plays a client with
 * {@link ClientMirror} (the real delta packets and client apply code) and
 * after every command compares the client's screen with the server's,
 * writing both to {@code screenshots/} as PNGs.
 *
 * <p>The two cases differ only in whether block-entity data packets arrive
 * too. On the server, typing calls {@code sendBlockUpdated}, and in a tick
 * the chunk broadcast (which sends that packet) runs before block entities
 * tick (which queues the delta), so a block update carrying the new screen
 * reaches the client before the delta computed against the old one. The
 * cases deliver them in that order.
 */
@GameTestHolder(DisplaySyncTests.NS)
@PrefixGameTestTemplate(false)
public final class DisplaySyncTests {
    static final String NS = "ecm_sync";
    private static final String NODE = "sw1";
    private static final String COMMAND = "vlans show";
    private static final int ROUNDS = 3;

    /** Deltas only: the protocol on its own. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".deltas_only")
    public static void deltas_only_client_matches_server(GameTestHelper h) {
        new Repro(h, "deltas_only_client_matches_server", false).start();
    }

    /** Deltas plus the block-entity update each keystroke triggers (the in-game case). */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".block_updates")
    public static void block_updates_client_matches_server(GameTestHelper h) {
        new Repro(h, "block_updates_client_matches_server", true).start();
    }

    private static final class Repro {
        private enum Phase { SCENARIO, TYPE, WAIT, DONE }

        private final GameTestHelper h;
        private final String name;
        private final boolean blockUpdates;
        private final ScenarioRun run;
        private Phase phase = Phase.SCENARIO;
        private ClientMirror client;
        private int round = 0;
        private String lastScreen;
        private long stableSince;
        private String failure;

        Repro(GameTestHelper h, String name, boolean blockUpdates) {
            this.h = h;
            this.name = name;
            this.blockUpdates = blockUpdates;
            Scenario sc = SwitchScenarios.ALL.get("switch_vlans");
            this.run = TestDriver.build(h, sc, name);
        }

        void start() {
            TestDriver.drive(h, NS, name, this::tick, () -> failure);
        }

        private TerminalBlockEntity sw() {
            return h.getLevel().getBlockEntity(run.abs(run.scenario().nodes.get(NODE).pos())) instanceof TerminalBlockEntity t ? t : null;
        }

        private boolean tick() {
            switch (phase) {
                case SCENARIO -> {
                    ScenarioRun.State st = run.tick();
                    if (st == ScenarioRun.State.FAILED) failure = "scenario failed: " + run.failure();
                    if (st != ScenarioRun.State.PASSED) return false;
                    // Open sw1's terminal: a keyframe.
                    client = new ClientMirror(sw());
                    client.pump();
                    shot("00-opened");
                    phase = Phase.TYPE;
                }
                case TYPE -> {
                    round++;
                    sw().onStringInput(COMMAND + "\n"); // what TerminalInputPacket's handler calls
                    lastScreen = null;
                    phase = Phase.WAIT;
                }
                case WAIT -> {
                    String scr = ScenarioRun.screen(sw().getDisplay());
                    long now = System.currentTimeMillis();
                    if (!scr.equals(lastScreen)) {
                        lastScreen = scr;
                        stableSince = now;
                        return false;
                    }
                    // The command's output and the next prompt are on screen, and it has settled.
                    if (!DONE_RE.matcher(after(scr)).find() || now - stableSince < 300) return false;
                    if (blockUpdates) client.blockEntityUpdate(h.getLevel().registryAccess());
                    client.pump();
                    String shown = ScenarioRun.screen(client.display());
                    shot(String.format("%02d-after-%s", round, COMMAND.replace(' ', '-')));
                    if (!shown.equals(scr)) {
                        failure = "round " + round + ": the client's screen differs from the server's after '"
                                + COMMAND + "'\n--- server ---\n" + scr.stripTrailing()
                                + "\n--- client ---\n" + shown.stripTrailing();
                        return false;
                    }
                    phase = round < ROUNDS ? Phase.TYPE : Phase.DONE;
                }
                case DONE -> {
                    return true;
                }
            }
            return false;
        }

        private static final Pattern DONE_RE =
                Pattern.compile("Unknown command: vlans[\\s\\S]*^/ >$", Pattern.MULTILINE);

        /** Rows after the last echo of the command, trailing spaces stripped. */
        private static String after(String screen) {
            String[] rows = screen.split("\n", -1);
            StringBuilder sb = new StringBuilder();
            boolean seen = false;
            for (String r : rows) {
                String t = r.stripTrailing();
                if (t.endsWith(COMMAND)) {
                    sb.setLength(0);
                    seen = true;
                    continue;
                }
                if (seen) sb.append(t).append('\n');
            }
            return sb.toString();
        }

        private void shot(String step) {
            ScreenCapture.write(name + "-" + step,
                    name + " - " + step + " (" + NODE + "; red = row differs from the server)",
                    List.of(new ScreenCapture.Panel("server screen", sw().getDisplay()),
                            new ScreenCapture.Panel("client screen", client.display())));
        }
    }

    private DisplaySyncTests() {}
}
//?}
