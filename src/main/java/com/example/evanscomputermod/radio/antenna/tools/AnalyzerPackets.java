package com.example.evanscomputermod.radio.antenna.tools;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.antenna.Antenna;
import com.example.evanscomputermod.radio.antenna.AntennaManager;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;

/**
 * Server to client: the antenna analyzer screen's data (sneak + right-click a
 * feed point with the analyzer). One packet carries the SWR curve across the
 * antenna's band; while the solve is pending the estimate is sent at once and
 * the solved curve follows.
 */
public final class AnalyzerPackets {
    /** Points across the plotted span. */
    public static final int POINTS = 61;
    private static final int MAX_POINTS = 512;
    private static final int MAX_TEXT = 512;

    private AnalyzerPackets() {}

    public static void register(RegisterPayloadHandlersEvent event) {
        event.registrar("1").playToClient(Plot.TYPE, Plot.STREAM_CODEC, (p, ctx) -> ctx.enqueueWork(
                () -> com.example.evanscomputermod.radio.antenna.tools.client.AntennaAnalyzerScreen.show(p)));
    }

    /**
     * Everything the screen shows. {@code hz} empty means no antenna (the
     * summary says why).
     */
    public record Plot(BlockPos pos, String summary, String limits, boolean pending, double resonantHz,
                       double bandLowHz, double bandHighHz, double[] hz, double[] swr) implements CustomPacketPayload {
        public static final Type<Plot> TYPE = new Type<>(EvansComputerMod.id("antenna_analyzer_plot"));
        public static final StreamCodec<FriendlyByteBuf, Plot> STREAM_CODEC = StreamCodec.of(Plot::write, Plot::read);

        static void write(FriendlyByteBuf buf, Plot p) {
            buf.writeBlockPos(p.pos);
            buf.writeUtf(clip(p.summary), MAX_TEXT);
            buf.writeUtf(clip(p.limits), MAX_TEXT);
            buf.writeBoolean(p.pending);
            buf.writeDouble(p.resonantHz);
            buf.writeDouble(p.bandLowHz);
            buf.writeDouble(p.bandHighHz);
            buf.writeVarInt(p.hz.length);
            for (int i = 0; i < p.hz.length; i++) {
                buf.writeDouble(p.hz[i]);
                buf.writeDouble(p.swr[i]);
            }
        }

        static Plot read(FriendlyByteBuf buf) {
            BlockPos pos = buf.readBlockPos();
            String summary = buf.readUtf(MAX_TEXT), limits = buf.readUtf(MAX_TEXT);
            boolean pending = buf.readBoolean();
            double res = buf.readDouble(), lo = buf.readDouble(), hi = buf.readDouble();
            int n = buf.readVarInt();
            if (n < 0 || n > MAX_POINTS) throw new IllegalArgumentException("bad analyzer point count " + n);
            double[] hz = new double[n], swr = new double[n];
            for (int i = 0; i < n; i++) {
                hz[i] = buf.readDouble();
                swr[i] = buf.readDouble();
            }
            return new Plot(pos, summary, limits, pending, res, lo, hi, hz, swr);
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    private static String clip(String s) {
        if (s == null) return "";
        return s.length() > MAX_TEXT / 2 ? s.substring(0, MAX_TEXT / 2) : s;
    }

    /** The screen's data for the antenna on {@code feed}, from the cache (never solves). */
    public static Plot build(Level level, Antenna a) {
        double[] span = AntennaToolsData.analyzerSpan(a.report());
        double[] hz = span == null ? new double[0] : AntennaToolsData.frequencies(span[0], span[1], POINTS);
        double[] swr = AntennaToolsData.swrCurve(a.report(), hz);
        String limits = a.present() ? (String) AntennaPeripheral.powerLimit(level, a, AntennaPeripheral.transmitter(level, a.feed())).get("text") : "";
        return new Plot(a.feed(), a.summary(), limits, a.pending(), a.resonantHz(), a.report().swrBandLowHz(),
                a.report().swrBandHighHz(), hz, swr);
    }

    /** Sends the plot now and, if the solve is pending, the solved plot when it lands. */
    public static void open(ServerPlayer player, Level level, BlockPos feed) {
        Antenna a = AntennaManager.get(level, feed);
        PacketDistributor.sendToPlayer(player, build(level, a));
        if (a.pending()) {
            AntennaManager.whenSolved(level, feed).thenAccept(solved -> {
                if (!player.isRemoved()) PacketDistributor.sendToPlayer(player, build(level, solved));
            });
        }
    }
}
//?}
