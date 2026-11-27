package com.example.evanscomputermod.command;

import com.example.evanscomputermod.EcmConfig;
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.api.repo.RepoLinks;
import com.example.evanscomputermod.computer.repo.RepoServers;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.context.CommandContext;
import com.mojang.brigadier.exceptions.CommandSyntaxException;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.coordinates.BlockPosArgument;
import net.minecraft.core.BlockPos;
import net.minecraft.network.chat.Component;
//? if >=26.1 {
import net.minecraft.server.permissions.PermissionCheck;
import net.minecraft.server.permissions.Permissions;
//?}

/**
 * The package repository's server settings, for operators (docs/REPO.md).
 *
 * <pre>
 *   /ecm repo status          how computers reach the repository here
 *   /ecm repo grant &lt;pos&gt;     make the computer at pos a repository server
 *   /ecm repo revoke &lt;pos&gt;
 *   /ecm repo list            the repository servers
 * </pre>
 *
 * A repository server may use the server's own link to the official repository
 * when {@code repo.access = datacenter}; it mirrors what it chooses
 * ({@code pkg mirror}) and offers it on the game's network ({@code pkg serve}).
 */
public final class RepoCommand {
    private RepoCommand() {}

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(
                Commands.literal("ecm")
                        //? if >=26.1 {
                        .requires(Commands.hasPermission(new PermissionCheck.Require(Permissions.COMMANDS_GAMEMASTER)))
                        //?} else
                        /*.requires(src -> src.hasPermission(2))*/
                        .then(Commands.literal("repo")
                                .then(Commands.literal("status").executes(RepoCommand::status))
                                .then(Commands.literal("list").executes(RepoCommand::list))
                                .then(Commands.literal("grant")
                                        .then(Commands.argument("pos", BlockPosArgument.blockPos()).executes(c -> change(c, true))))
                                .then(Commands.literal("revoke")
                                        .then(Commands.argument("pos", BlockPosArgument.blockPos()).executes(c -> change(c, false))))));
    }

    private static void say(CommandContext<CommandSourceStack> c, String text) {
        c.getSource().sendSuccess(() -> Component.literal(text), false);
    }

    private static int status(CommandContext<CommandSourceStack> c) {
        String access = EcmConfig.repoAccess();
        boolean link = RepoLinks.get() != null;
        say(c, "Package repository: " + EcmConfig.repoUrl() + " (repo.access = " + access + ")");
        say(c, switch (access) {
            case "off" -> "  Computers do not reach it. Repositories inside the game (pkg serve) still work.";
            case "open" -> link
                    ? "  Every computer reaches it through the server; no in-game network is needed."
                    : "  Nothing reaches it: open needs the internet add-on (evanscomputermod-internet) on the server.";
            case "datacenter" -> link
                    ? "  Only repository servers reach it (" + RepoServers.list(c.getSource().getServer()).size() + " here); others get packages from one of them."
                    : "  Nothing reaches it: datacenter needs the internet add-on (evanscomputermod-internet) on the server.";
            default -> "  A computer reaches it over its own network: it needs a route to an Internet Gateway.";
        });
        say(c, EcmConfig.repoKey().length() == 64
                ? "  Signing key: " + EcmConfig.repoKey().substring(0, 16) + "... (computers get the repository in /etc/pkg/repos when they boot)"
                : "  No signing key configured (repo.key): computers are not given the repository. Set it in config/evanscomputermod-common.toml.");
        say(c, "  Community channel: " + (EcmConfig.repoAllowCommunity() ? "offered" : "not offered")
                + "; publishing through the server's link: " + (EcmConfig.repoAllowPublish() ? "allowed" : "not allowed"));
        return 1;
    }

    private static int list(CommandContext<CommandSourceStack> c) {
        var ids = RepoServers.list(c.getSource().getServer());
        if (ids.isEmpty()) say(c, "No repository servers. Name one with /ecm repo grant <pos> (the position of its Terminal).");
        for (var id : ids) say(c, "  " + id);
        return ids.size();
    }

    private static int change(CommandContext<CommandSourceStack> c, boolean grant) throws CommandSyntaxException {
        BlockPos pos = BlockPosArgument.getLoadedBlockPos(c, "pos");
        if (!(c.getSource().getLevel().getBlockEntity(pos) instanceof IComputerHost host) || host.getComputerId() == null) {
            c.getSource().sendFailure(Component.literal("There is no computer at " + pos.toShortString() + "."));
            return 0;
        }
        var server = c.getSource().getServer();
        boolean changed = grant ? RepoServers.grant(server, host.getComputerId()) : RepoServers.revoke(server, host.getComputerId());
        say(c, (grant
                ? (changed ? "The computer at " + pos.toShortString() + " is now a repository server." : "It already is a repository server.")
                : (changed ? "The computer at " + pos.toShortString() + " is no longer a repository server." : "It was not a repository server."))
                + (changed ? " Restart it (break and place, or reboot) to update its /etc/pkg/repos." : ""));
        if (grant && !EcmConfig.repoAccess().equals("datacenter")) {
            say(c, "Note: repo.access is " + EcmConfig.repoAccess() + "; repository servers only matter when it is datacenter.");
        }
        return 1;
    }
}
