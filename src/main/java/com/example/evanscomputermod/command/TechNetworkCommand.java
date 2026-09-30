package com.example.evanscomputermod.command;

//? if <=1.21.1 {
import com.example.evanscomputermod.computer.*;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;

import net.minecraft.commands.*;
import net.minecraft.network.chat.Component;
import net.minecraft.world.level.levelgen.Heightmap;

public final class TechNetworkCommand {
    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        var root = Commands.literal("ecm").requires(s -> s.hasPermission(2));
        var village = Commands.literal("techvillage");
        village.then(
                Commands.literal("list")
                        .executes(
                                c -> {
                                    var d = data(c.getSource());
                                    for (var v : d.villages)
                                        say(
                                                c.getSource(),
                                                "Village "
                                                        + v.number()
                                                        + " AS "
                                                        + (65000 + v.number())
                                                        + " at "
                                                        + v.x()
                                                        + ", "
                                                        + v.z());
                                    return d.villages.size();
                                }));
        for (String action : new String[] {"tp", "info"})
            village.then(
                    Commands.literal(action)
                            .then(
                                    Commands.argument("number", IntegerArgumentType.integer(1, 10))
                                            .executes(
                                                    c -> {
                                                        var s = c.getSource();
                                                        int i =
                                                                IntegerArgumentType.getInteger(
                                                                        c, "number");
                                                        var v = data(s).villages.get(i - 1);
                                                        if (action.equals("tp")) {
                                                            var p = s.getPlayerOrException();
                                                            var l = s.getServer().overworld();
                                                            l.getChunk(v.x() >> 4, v.z() >> 4);
                                                            p.teleportTo(
                                                                    l,
                                                                    v.x() + 24,
                                                                    l.getHeight(
                                                                                    Heightmap.Types
                                                                                            .MOTION_BLOCKING_NO_LEAVES,
                                                                                    v.x() + 24,
                                                                                    v.z() + 24)
                                                                            + 1,
                                                                    v.z() + 24,
                                                                    p.getYRot(),
                                                                    p.getXRot());
                                                        } else
                                                            say(
                                                                    s,
                                                                    "Village "
                                                                            + i
                                                                            + ": AS "
                                                                            + (65000 + i)
                                                                            + ", customer port"
                                                                            + " eth5: 100."
                                                                            + (64 + i)
                                                                            + ".2.1/24; server 100."
                                                                            + (64 + i)
                                                                            + ".0.10; router "
                                                                            + data(s).identity(
                                                                                            s.getServer()
                                                                                                    .overworld(),
                                                                                            i,
                                                                                            "isp.router"));
                                                        return 1;
                                                    })));
        root.then(village);
        var network = Commands.literal("net");
        network.then(
                Commands.literal("links")
                        .executes(
                                c -> {
                                    var d = data(c.getSource());
                                    for (int i = 1; i <= 10; i++) {
                                        int next = i == 10 ? 1 : i + 1;
                                        String name = WorldNetwork.linkName(i, next);
                                        boolean broken =
                                                d.brokenFiber.stream()
                                                        .anyMatch(
                                                                p ->
                                                                        name.equals(
                                                                                com.example
                                                                                        .evanscomputermod
                                                                                        .worldgen
                                                                                        .FiberWorld
                                                                                        .linkAt(
                                                                                                d,
                                                                                                net
                                                                                                        .minecraft
                                                                                                        .core
                                                                                                        .BlockPos
                                                                                                        .of(
                                                                                                                p))));
                                        say(
                                                c.getSource(),
                                                name
                                                        + ": "
                                                        + (d.cuts.contains(name) || broken
                                                                ? "CUT"
                                                                : "intact"));
                                    }
                                    return 10;
                                }));
        for (String action : new String[] {"cut", "repair"})
            network.then(
                    Commands.literal(action)
                            .then(
                                    Commands.argument("a", IntegerArgumentType.integer(1, 10))
                                            .then(
                                                    Commands.argument(
                                                                    "b",
                                                                    IntegerArgumentType.integer(
                                                                            1, 10))
                                                            .executes(
                                                                    c -> {
                                                                        int
                                                                                a =
                                                                                        IntegerArgumentType
                                                                                                .getInteger(
                                                                                                        c,
                                                                                                        "a"),
                                                                                b =
                                                                                        IntegerArgumentType
                                                                                                .getInteger(
                                                                                                        c,
                                                                                                        "b");
                                                                        if (Math.abs(a - b) != 1
                                                                                && Math.abs(a - b)
                                                                                        != 9) {
                                                                            say(
                                                                                    c.getSource(),
                                                                                    "Those villages"
                                                                                        + " are not"
                                                                                        + " adjacent.");
                                                                            return 0;
                                                                        }
                                                                        data(c.getSource())
                                                                                .link(
                                                                                        c.getSource()
                                                                                                .getServer()
                                                                                                .overworld(),
                                                                                        a,
                                                                                        b,
                                                                                        action
                                                                                                .equals(
                                                                                                        "repair"));
                                                                        say(
                                                                                c.getSource(),
                                                                                WorldNetwork
                                                                                                .linkName(
                                                                                                        a,
                                                                                                        b)
                                                                                        + " "
                                                                                        + action
                                                                                        + "; physical"
                                                                                        + " breaks"
                                                                                        + " must"
                                                                                        + " also be"
                                                                                        + " repaired.");
                                                                        return 1;
                                                                    }))));
        root.then(network);
        root.then(
                Commands.literal("headless")
                        .then(
                                Commands.literal("list")
                                        .executes(
                                                c -> {
                                                    int count = 0;
                                                    for (var h :
                                                            ComputerHost.list(
                                                                    c.getSource().getServer()))
                                                        if (h.isHeadless()) {
                                                            say(
                                                                    c.getSource(),
                                                                    h.getComputerId()
                                                                            + (h.isInfrastructure()
                                                                                    ? " infrastructure"
                                                                                    : " always-on"));
                                                            count++;
                                                        }
                                                    return count;
                                                })));
        dispatcher.register(root);
    }

    private static WorldNetwork data(CommandSourceStack s) {
        var d = WorldNetwork.get(s.getServer().overworld());
        d.plan(s.getServer().overworld());
        return d;
    }

    private static void say(CommandSourceStack s, String text) {
        s.sendSuccess(() -> Component.literal(text), false);
    }
}
//?}
