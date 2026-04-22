package com.example.evanscomputermod.command;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.RedstoneNetBlockEntity;
import com.example.evanscomputermod.computer.netdev.EcmProto;
import com.example.evanscomputermod.computer.netdev.NetDevice;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.IntegerArgumentType;
import com.mojang.brigadier.arguments.StringArgumentType;
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
import net.minecraft.world.level.block.entity.BlockEntity;

/**
 * Configuration command for Java-side network devices. Primary use today is
 * the Redstone Network Block; the subcommand structure is intentionally generic
 * so other {@link NetDevice} subclasses can be added later without rework.
 *
 * <pre>
 *   /ecmnetdev &lt;pos&gt; info
 *   /ecmnetdev &lt;pos&gt; ip &lt;cidr&gt;
 *   /ecmnetdev &lt;pos&gt; port &lt;number&gt;
 * </pre>
 */
public class NetDevCommand {

    public static void register(CommandDispatcher<CommandSourceStack> dispatcher) {
        dispatcher.register(
            Commands.literal("ecmnetdev")
                //? if >=26.1 {
                .requires(Commands.hasPermission(new PermissionCheck.Require(Permissions.COMMANDS_GAMEMASTER)))
                //?} else
                /*.requires(src -> src.hasPermission(2))*/
                .then(Commands.argument("pos", BlockPosArgument.blockPos())
                    .then(Commands.literal("info")
                        .executes(NetDevCommand::info))
                    .then(Commands.literal("ip")
                        .then(Commands.argument("cidr", StringArgumentType.word())
                            .executes(NetDevCommand::setIp)))
                    .then(Commands.literal("port")
                        .then(Commands.argument("number", IntegerArgumentType.integer(1, 65535))
                            .executes(NetDevCommand::setPort))))
        );
        EvansComputerMod.LOGGER.info("Registered /ecmnetdev command");
    }

    private static RedstoneNetBlockEntity resolveBe(CommandContext<CommandSourceStack> ctx) throws CommandSyntaxException {
        BlockPos pos = BlockPosArgument.getBlockPos(ctx, "pos");
        CommandSourceStack src = ctx.getSource();
        BlockEntity be = src.getLevel().getBlockEntity(pos);
        if (!(be instanceof RedstoneNetBlockEntity rbe)) {
            src.sendFailure(Component.literal("§cNo network device at " + pos.toShortString()));
            return null;
        }
        return rbe;
    }

    private static int info(CommandContext<CommandSourceStack> ctx) throws CommandSyntaxException {
        RedstoneNetBlockEntity be = resolveBe(ctx);
        if (be == null) return 0;
        CommandSourceStack src = ctx.getSource();
        int ip = be.getConfiguredIp();
        int prefix = be.getConfiguredPrefix();
        int port = be.getConfiguredPort();
        String ipStr = (prefix > 0)
                ? NetDevice.formatIp(ip) + "/" + prefix
                : "unconfigured";
        src.sendSystemMessage(Component.literal(
                "§7device id: §f" + be.getDeviceId() + "\n"
              + "§7ip:        §f" + ipStr + "\n"
              + "§7port:      §f" + port));
        return 1;
    }

    private static int setIp(CommandContext<CommandSourceStack> ctx) throws CommandSyntaxException {
        RedstoneNetBlockEntity be = resolveBe(ctx);
        if (be == null) return 0;
        CommandSourceStack src = ctx.getSource();
        String cidr = StringArgumentType.getString(ctx, "cidr");
        try {
            int[] parsed = NetDevice.parseCidr(cidr);
            be.configure(parsed[0], parsed[1]);
            src.sendSuccess(() -> Component.literal("§adevice configured: " + cidr), false);
            return 1;
        } catch (IllegalArgumentException e) {
            src.sendFailure(Component.literal("§cbad CIDR: " + e.getMessage()));
            return 0;
        }
    }

    private static int setPort(CommandContext<CommandSourceStack> ctx) throws CommandSyntaxException {
        RedstoneNetBlockEntity be = resolveBe(ctx);
        if (be == null) return 0;
        int port = IntegerArgumentType.getInteger(ctx, "number");
        be.configurePort(port);
        ctx.getSource().sendSuccess(() -> Component.literal("§aport set: " + port), false);
        return 1;
    }
}
