package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.RadioMedium;
import net.minecraft.server.MinecraftServer;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.server.ServerStartingEvent;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;

import java.util.function.Function;

/**
 * Owns the server's {@link RadioMedium}: created on server start, dropped on
 * stop. The airtime clock is game time (50 ms per tick) plus the wall-clock
 * time since the tick began, capped at one tick, so it is continuous and
 * monotonic even when the server lags.
 */
public final class RadioMediumHooks {

    private static volatile RadioMedium medium;
    private static volatile Function<MinecraftServer, RadioMedium> factory = RadioMediumHooks::basic;
    private static volatile long tickBaseMicros;
    private static volatile long tickWallNanos;
    private static volatile long lastMicros;

    private RadioMediumHooks() {}

    public static void register(IEventBus modBus) {
        NeoForge.EVENT_BUS.addListener((ServerStartingEvent e) -> start(e.getServer()));
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> medium = null);
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Pre e) -> {
            tickBaseMicros = e.getServer().overworld().getGameTime() * 50_000L;
            tickWallNanos = System.nanoTime();
        });
    }

    /** Replace how the medium is built (the in-world medium installs itself here). */
    public static void setFactory(Function<MinecraftServer, RadioMedium> f) {
        factory = f;
    }

    private static void start(MinecraftServer server) {
        tickBaseMicros = server.overworld().getGameTime() * 50_000L;
        tickWallNanos = System.nanoTime();
        lastMicros = 0;
        medium = factory.apply(server);
    }

    /** The running medium, or null when no server is running. */
    public static RadioMedium medium() {
        return medium;
    }

    /** World airtime clock, microseconds. */
    public static synchronized long clockMicros() {
        long within = Math.min(50_000L, (System.nanoTime() - tickWallNanos) / 1000);
        long now = Math.max(lastMicros, tickBaseMicros + within);
        lastMicros = now;
        return now;
    }

    private static RadioMedium basic(MinecraftServer server) {
        return new BasicRadioMedium(RadioMediumHooks::clockMicros, server.overworld().getSeed());
    }
}
//?}
