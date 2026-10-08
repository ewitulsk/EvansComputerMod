package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.phys.Noise;
import com.mojang.brigadier.CommandDispatcher;
import com.mojang.brigadier.arguments.DoubleArgumentType;
import com.mojang.brigadier.context.CommandContext;
import net.minecraft.commands.CommandSourceStack;
import net.minecraft.commands.Commands;
import net.minecraft.commands.arguments.coordinates.Vec3Argument;
import net.minecraft.network.chat.Component;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.phys.Vec3;

import java.util.Locale;

/**
 * {@code /ecm radio link <x y z> <x y z> <freqMHz>}: traces one path now and
 * prints the loss breakdown (free space, walls, sub-level hulls, diffraction,
 * ground, skywave) plus the noise floor in 20 MHz / 3 kHz.
 */
public final class RadioLinkCommand {

    private RadioLinkCommand() {}

    public static void register(CommandDispatcher<CommandSourceStack> d) {
        d.register(Commands.literal("ecm").requires(s -> s.hasPermission(2))
                .then(Commands.literal("radio").then(Commands.literal("link")
                        .then(Commands.argument("from", Vec3Argument.vec3())
                                .then(Commands.argument("to", Vec3Argument.vec3())
                                        .then(Commands.argument("freqMHz", DoubleArgumentType.doubleArg(0.003, 100_000))
                                                .executes(RadioLinkCommand::run)))))));
    }

    private static int run(CommandContext<CommandSourceStack> ctx) {
        ServerLevel level = ctx.getSource().getLevel();
        Vec3 a = Vec3Argument.getVec3(ctx, "from"), b = Vec3Argument.getVec3(ctx, "to");
        double f = DoubleArgumentType.getDouble(ctx, "freqMHz") * 1e6;
        WorldRadioMedium m = WorldMediumContent.medium();
        RfWorld w = WorldMediumContent.rfWorld(level);
        if (m == null || w == null) {
            ctx.getSource().sendFailure(Component.literal("The in-world radio medium isn't running."));
            return 0;
        }
        String dim = level.dimension().location().toString();
        var c = new WorldRadioMedium.Conditions(level.getDayTime(), level.isThundering(), level.dimensionType().hasSkyLight());
        PathTracer.Result r = m.traceNow(w, Pose.at(dim, a.x, a.y, a.z), Pose.at(dim, b.x, b.y, b.z), f, c);
        double bw = f >= 1e9 ? 20e6 : 3e3;
        double noise = Noise.floorDbm(bw, 6, f, Noise.Environment.RURAL, c.dayTime(), c.thundering());
        String msg = String.format(Locale.ROOT,
                "%.1f m @ %.3f MHz: total %.1f dB (%s%s)%n free space %.1f, walls %.1f (hulls %.1f), diffraction %.1f, "
                        + "ground %+.1f, skywave %s%n heights %.1f/%.1f m over %s; noise floor %.1f dBm in %s; "
                        + "links cached %d, queued %d",
                r.distanceM(), f / 1e6, r.totalDb(), r.mode(), r.lineOfSight() ? ", LOS" : ", NLOS",
                r.freeSpaceDb(), r.obstructionDb(), r.volumeDb(), r.diffractionDb(), r.groundExcessDb(),
                Double.isInfinite(r.skywaveDb()) ? "none" : String.format(Locale.ROOT, "%.1f", r.skywaveDb()),
                r.txHeightM(), r.rxHeightM(), r.groundName(), noise, bw >= 1e6 ? "20 MHz" : "3 kHz",
                m.cachedLinks(), m.queuedLinks());
        ctx.getSource().sendSuccess(() -> Component.literal(msg), false);
        return 1;
    }
}
//?}
