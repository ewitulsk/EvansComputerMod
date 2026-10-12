package com.example.evanscomputermod.command;

import com.example.evanscomputermod.computer.ComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
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
 * §14, §17): for every computer whose compositor runs, whether its displays
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

    private static int stats(CommandSourceStack src) {
        int n = 0;
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
            for (Map.Entry<UiSceneHub.ViewerKey, UiSceneHub.Viewer> e : hub.viewers().entrySet()) {
                ServerPlayer p = src.getServer().getPlayerList().getPlayer(e.getKey().player());
                UiSceneHub.Viewer v = e.getValue();
                say(src, "  " + (p == null ? e.getKey().player().toString() : p.getName().getString()) + " display "
                        + e.getKey().displayId() + ": " + v.bytes() + " B in " + v.messages() + " messages (" + v.steps()
                        + " steps, " + v.keyframes() + " keyframes, " + v.resources() + " images), at gen " + v.sentGeneration());
            }
        }
        if (n == 0) say(src, "no computer runs the GUI");
        return n;
    }

    private static void say(CommandSourceStack s, String text) {
        s.sendSuccess(() -> Component.literal(text), false);
    }
}
