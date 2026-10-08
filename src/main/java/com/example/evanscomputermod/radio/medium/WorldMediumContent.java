package com.example.evanscomputermod.radio.medium;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.RadioConfig;
import com.example.evanscomputermod.radio.api.RadioMedium;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.chunk.LevelChunk;
import net.minecraft.world.level.levelgen.Heightmap;
import net.neoforged.bus.api.IEventBus;
import net.neoforged.neoforge.common.NeoForge;
import net.neoforged.neoforge.event.RegisterCommandsEvent;
import net.neoforged.neoforge.event.level.BlockEvent;
import net.neoforged.neoforge.event.level.ChunkEvent;
import net.neoforged.neoforge.event.server.ServerStoppedEvent;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.registries.datamaps.DataMapsUpdatedEvent;
import net.neoforged.neoforge.registries.datamaps.RegisterDataMapTypesEvent;

import java.util.HashMap;
import java.util.Map;
import java.util.concurrent.ConcurrentLinkedQueue;

/**
 * Installs {@link WorldRadioMedium} as the server's medium and feeds it the
 * world: the {@code rf_attenuation} data map, the server tick (moves, link
 * recompute within {@code RadioConfig.raysPerTick}), block changes
 * ({@link BlockEvent.NeighborNotifyEvent}: every {@code setBlock} with neighbour
 * updates), chunk load/unload (RF summaries), Sable sub-level motion, and the
 * {@code /ecm radio link} debug command.
 */
public final class WorldMediumContent {

    private static volatile Glue glue;

    private WorldMediumContent() {}

    public static void register(IEventBus modBus) {
        modBus.addListener((RegisterDataMapTypesEvent e) -> e.register(RfAttenuation.TYPE));
        RadioMediumHooks.setFactory(WorldMediumContent::create);
        NeoForge.EVENT_BUS.addListener((DataMapsUpdatedEvent e) -> RfAttenuation.clearCache());
        NeoForge.EVENT_BUS.addListener((ServerStoppedEvent e) -> glue = null);
        NeoForge.EVENT_BUS.addListener((ServerTickEvent.Post e) -> {
            Glue g = glue;
            if (g != null) g.tick();
        });
        NeoForge.EVENT_BUS.addListener((BlockEvent.NeighborNotifyEvent e) -> {
            Glue g = glue;
            if (g != null && e.getLevel() instanceof ServerLevel level) g.blockChanged(level, e.getPos());
        });
        NeoForge.EVENT_BUS.addListener((ChunkEvent.Unload e) -> {
            Glue g = glue;
            if (g != null && e.getLevel() instanceof ServerLevel level && e.getChunk() instanceof LevelChunk c)
                g.chunkEvents.add(new ChunkChange(level, c, false));
        });
        NeoForge.EVENT_BUS.addListener((ChunkEvent.Load e) -> {
            Glue g = glue;
            if (g != null && e.getLevel() instanceof ServerLevel level && e.getChunk() instanceof LevelChunk c)
                g.chunkEvents.add(new ChunkChange(level, c, true));
        });
        NeoForge.EVENT_BUS.addListener((RegisterCommandsEvent e) -> RadioLinkCommand.register(e.getDispatcher()));
    }

    /** The running in-world medium, or null. */
    public static WorldRadioMedium medium() {
        return RadioMediumHooks.medium() instanceof WorldRadioMedium w ? w : null;
    }

    /** The world view of a level for tracing (server thread), or null when no medium runs. */
    public static RfWorld rfWorld(ServerLevel level) {
        Glue g = glue;
        return g == null ? null : g.world(level);
    }

    private static RadioMedium create(MinecraftServer server) {
        WorldRadioMedium m = new WorldRadioMedium(RadioMediumHooks::clockMicros, server.overworld().getSeed());
        glue = new Glue(server, m);
        return m;
    }

    private record ChunkChange(ServerLevel level, LevelChunk chunk, boolean load) {}

    private static final class Glue implements WorldRadioMedium.WorldAccess {
        final MinecraftServer server;
        final WorldRadioMedium medium;
        final Map<String, ServerLevel> levels = new HashMap<>();
        final Map<String, LevelRfWorld> worlds = new HashMap<>();
        final Map<String, LevelRfWorld.Summaries> summaries = new HashMap<>();
        final Map<String, SableRf.Tracker> trackers = new HashMap<>();
        final ConcurrentLinkedQueue<ChunkChange> chunkEvents = new ConcurrentLinkedQueue<>();

        Glue(MinecraftServer server, WorldRadioMedium medium) {
            this.server = server;
            this.medium = medium;
        }

        static String dim(ServerLevel level) {
            return level.dimension().location().toString();
        }

        void tick() {
            if (RadioMediumHooks.medium() != medium) return;
            for (ServerLevel l : server.getAllLevels()) levels.put(dim(l), l);
            for (ChunkChange c; (c = chunkEvents.poll()) != null; ) {
                String d = dim(c.level());
                LevelRfWorld.Summaries s = summaries.computeIfAbsent(d, k -> new LevelRfWorld.Summaries());
                if (c.load()) s.remove(c.chunk().getPos().toLong());
                else s.put(c.chunk());
                medium.onChunkChanged(d, c.chunk().getPos().x, c.chunk().getPos().z,
                        c.level().getMinSection(), c.level().getMaxSection() - 1);
            }
            long tick = server.overworld().getGameTime();
            for (ServerLevel l : levels.values()) {
                String d = dim(l);
                trackers.computeIfAbsent(d, k -> new SableRf.Tracker()).tick(l, tick, RadioConfig.sableRecomputeMetres(),
                        RadioConfig.sableRecomputeRadians(), RadioConfig.sableMinRecomputeTicks(),
                        (x0, y0, z0, x1, y1, z1) -> medium.onRegionChanged(d, x0, y0, z0, x1, y1, z1));
            }
            medium.tick(tick, this);
        }

        void blockChanged(ServerLevel level, net.minecraft.core.BlockPos p) {
            boolean surface = p.getY() >= level.getHeight(Heightmap.Types.MOTION_BLOCKING, p.getX(), p.getZ()) - 2;
            medium.onBlockChanged(dim(level), p.getX(), p.getY(), p.getZ(), surface);
        }

        LevelRfWorld world(ServerLevel level) {
            String d = dim(level);
            return worlds.computeIfAbsent(d, k -> new LevelRfWorld(level,
                    summaries.computeIfAbsent(k, x -> new LevelRfWorld.Summaries())));
        }

        @Override
        public RfWorld world(String dimension) {
            ServerLevel l = levels.get(dimension);
            return l == null ? null : world(l);
        }

        @Override
        public WorldRadioMedium.Conditions conditions(String dimension) {
            ServerLevel l = levels.get(dimension);
            if (l == null) return null;
            return new WorldRadioMedium.Conditions(l.getDayTime(), l.isThundering(), l.dimensionType().hasSkyLight());
        }

        @Override
        public Object subLevelOf(String dimension, double x, double y, double z) {
            ServerLevel l = levels.get(dimension);
            return l == null ? null : SableRf.subLevelAt(l, x, y, z);
        }
    }
}
//?}
