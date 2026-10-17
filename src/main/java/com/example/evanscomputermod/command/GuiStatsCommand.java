package com.example.evanscomputermod.command;

import com.example.evanscomputermod.computer.ComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.UiClients;
import com.example.evanscomputermod.computer.ui.UiManager;
import com.example.evanscomputermod.computer.ui.UiSceneHub;
import com.mojang.brigadier.CommandDispatcher;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
//? if >=26.1 {
import net.minecraft.server.permissions.PermissionCheck;
import net.minecraft.server.permissions.Permissions;
//?}
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerPlayer;

import java.util.Map;

/**
 * {@code /ecm gui stats}: GUI client rendering at a glance (UI_PROTOCOL.md
 * §14, §18): for every computer whose compositor runs, whether its displays
 * are streamed as scenes, and the bytes each scene viewer has cost so far;
 * plus which online players' clients render scenes.
 */
public final class GuiStatsCommand {
    private GuiStatsCommand() {}

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(Commands.literal("ecm")
                //? if >=26.1 {
                .requires(Commands.hasPermission(new PermissionCheck.Require(Permissions.COMMANDS_GAMEMASTER)))
                //?} else
                /*.requires(src -> src.hasPermission(2))*/
                .then(Commands.literal("gui").then(Commands.literal("stats").executes(c -> stats(c.getSource())))));
    }

    private static String ms(long nanos, long count) {
        return count == 0 ? "-" : String.format(java.util.Locale.ROOT, "%.2f ms", nanos / 1e6 / count);
    }

    private static String link(double[] l) {
        return String.format(java.util.Locale.ROOT, "rtt %.0f ms, jitter %.1f ms, %.0f kbit/s", l[0], l[1], l[2]);
    }

    private static int stats(CommandSourceStack src) {
        int n = 0;
        // The display pump (DISPLAY_PIPELINE_SPEC.md section 12): its passes, and display work left on the server thread.

        say(src, "display pump: " + com.example.evanscomputermod.computer.pipeline.DisplayPump.threads() + " threads, "
                + com.example.evanscomputermod.computer.pipeline.DisplayPump.passes() + " passes, mean "
                + ms(com.example.evanscomputermod.computer.pipeline.DisplayPump.passNanos(),
                com.example.evanscomputermod.computer.pipeline.DisplayPump.passes())
                + ", longest " + ms(com.example.evanscomputermod.computer.pipeline.DisplayPump.maxPassNanos(), 1)
                + "; on the server thread " + ms(com.example.evanscomputermod.computer.pipeline.DisplayPump.serverThreadNanos(), 1));
        for (ComputerHost h : ComputerHost.list(src.getServer())) {
            ComputerInstance ci = h.instance();
            var host = ci == null ? null : ci.currentHost();
            if (host == null) continue;
            for (var e : host.syncHubs().entrySet()) {
                var hub = e.getValue();
                if (hub.viewerCount() == 0 && hub.deltaBuilds() == 0) continue;
                var dev = ci.displayDevice(e.getKey() == 1 ? ComputerInstance.GFX_TARGET_SCREEN : ComputerInstance.GFX_TARGET_TERMINAL);
                var q = dev == null ? null : dev.queue();
                say(src, "computer " + h.getComputerId().toString().substring(0, 8) + " display " + e.getKey() + " tiles: "
                        + hub.deltaBuilds() + " deltas, " + hub.keyframeBuilds() + " keyframes, " + hub.timedBuilds() + " timed, "
                        + hub.jpegBuilds() + " JPEG (rung " + hub.rung() + "), " + hub.refines() + " refines, "
                        + hub.framesSkipped() + " frames skipped, " + hub.bytesSent() + " B"
                        + (q == null ? "" : "; queue: " + q.framesQueued() + " queued, " + q.framesShown() + " shown, "
                        + q.framesLate() + " late, " + q.framesFlushed() + " dropped"));
                for (java.util.UUID v : hub.viewerIds()) {
                    double[] l = hub.viewerStats(v);
                    if (l == null) continue;
                    ServerPlayer p = src.getServer().getPlayerList().getPlayer(v);
                    say(src, "  " + (p == null ? v.toString().substring(0, 8) : p.getName().getString()) + ": " + link(l)
                            + ", " + (int) l[3] + " in flight, " + (int) l[4] + " timeouts, " + hub.bytesSent(v) + " B");
                }
            }
        }
        for (ServerPlayer p : src.getServer().getPlayerList().getPlayers()) {
            say(src, "player " + p.getName().getString() + ": " + (UiClients.canRender(p.getUUID()) ? "renders scenes" : "tiles only"));
        }
        for (ComputerHost h : ComputerHost.list(src.getServer())) {
            ComputerInstance ci = h.instance();
            UiManager ui = ci == null ? null : ci.getUiManager();
            if (ui == null || !ui.isRunning()) continue;
            n++;
            UiSceneHub hub = ui.sceneHub();
            say(src, "computer " + h.getComputerId().toString().substring(0, 8) + ": GUI " + (ui.guiVisible() ? "shown" : "hidden")
                    + ", scene stream own=" + ui.sceneEligible(0) + " screen=" + ui.sceneEligible(1)
                    + ", " + hub.totalBytes() + " B sent, " + hub.keyframesBuilt() + " keyframes, gen " + hub.headGeneration());
            long[] tc = hub.timedCounters();
            Compositor comp = ui.compositor();
            long[] cq = comp == null ? new long[4] : comp.timedCounters();
            say(src, "  timed frames: " + tc[0] + " built, " + tc[1] + " JPEG (rung " + hub.rung() + "), " + tc[2]
                    + " skipped for slow viewers, " + tc[3] + " refines; queued " + cq[0] + ", shown " + cq[1] + ", replaced "
                    + cq[2] + ", flushed " + cq[3] + (comp == null ? "" : "; composites waited " + comp.compositesSkipped()));
            for (Map.Entry<UiSceneHub.ViewerKey, UiSceneHub.Viewer> e : hub.viewers().entrySet()) {
                ServerPlayer p = src.getServer().getPlayerList().getPlayer(e.getKey().player());
                UiSceneHub.Viewer v = e.getValue();
                say(src, "  " + (p == null ? e.getKey().player().toString() : p.getName().getString()) + " display "
                        + e.getKey().displayId() + ": " + v.bytes() + " B in " + v.messages() + " messages (" + v.steps()
                        + " steps, " + v.keyframes() + " keyframes, " + v.resources() + " images, " + v.timed() + " timed), at gen "
                        + v.sentGeneration() + "; " + link(v.link()) + ", " + v.timeouts() + " timeouts");
            }
        }
        if (n == 0) say(src, "no computer runs the GUI");
        return n;
    }

    private static void say(CommandSourceStack s, String text) {
        s.sendSuccess(() -> Component.literal(text), false);
    }
}
