package com.example.evanscomputermod.upload;

import com.example.evanscomputermod.api.upload.UploadBackend;
import com.example.evanscomputermod.api.upload.UploadSink;
import com.example.evanscomputermod.api.upload.UploadStatus;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.common.util.FakePlayer;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;
import java.util.HashMap;
import java.util.Iterator;
import java.util.Map;
import java.util.UUID;

/**
 * The server side: the {@link UploadBackend} the main mod calls, and the
 * handlers of what players' games send. Everything runs on the server thread.
 *
 * <p>An upload waits for the player to pick a file for at most
 * {@link #PICK_TIMEOUT_TICKS}, and once bytes flow it fails when none come
 * for {@link #IDLE_TIMEOUT_TICKS} or the player leaves. Packets for a ticket
 * are taken only from the player it was sent to.
 */
public final class ServerUploads implements UploadBackend {
    private static final Logger LOGGER = LoggerFactory.getLogger(ServerUploads.class);
    /** 10 minutes to pick a file. */
    static final int PICK_TIMEOUT_TICKS = 20 * 60 * 10;
    /** 30 s without a chunk while receiving. */
    static final int IDLE_TIMEOUT_TICKS = 20 * 30;

    static final ServerUploads INSTANCE = new ServerUploads();

    private record Pending(UploadSink sink, UUID player, long[] lastTick, boolean[] receiving) {}

    private final Map<Integer, Pending> pending = new HashMap<>();
    /** Scenario players (FakePlayers, no game of their own) with the file they "pick" (simulatePick). */
    private final Map<UUID, Path> simulated = new HashMap<>();
    private long tick;
    private net.minecraft.server.MinecraftServer currentServer;

    private ServerUploads() {}

    @Override
    public int apiVersion() {
        return CURRENT_API_VERSION;
    }

    @Override
    public String name() {
        return "evanscomputermod-upload";
    }

    @Override
    public boolean canUpload(ServerPlayer player) {
        if (player instanceof FakePlayer) return simulated.containsKey(player.getUUID());
        return Compat.hasChannel(player, UploadPackets.Pick.TYPE);
    }

    @Override
    public boolean permitted(ServerPlayer player) {
        return UploadConfig.enabled() && (!UploadConfig.operatorsOnly() || Compat.isOperator(player));
    }

    @Override
    public long maxFileBytes() {
        return UploadConfig.maxFileBytes();
    }

    @Override
    public long maxComputerBytes() {
        return UploadConfig.maxComputerBytes();
    }

    @Override
    public void start(ServerPlayer player, UploadSink sink) {
        pending.put(sink.ticket(), new Pending(sink, player.getUUID(), new long[]{tick}, new boolean[]{false}));
        Path file = player instanceof FakePlayer ? simulated.remove(player.getUUID()) : null;
        if (file != null) {
            simulate(player, sink.ticket(), file);
            return;
        }
        Compat.sendToPlayer(player, new UploadPackets.Pick(sink.ticket(), sink.computerKind(), sink.destination(), maxFileBytes()));
    }

    @Override
    public void cancel(UploadSink sink) {
        Pending p = pending.remove(sink.ticket());
        if (p == null || currentServer == null) return;
        // The program stopped waiting: the game stops sending (and says so).
        ServerPlayer player = currentServer.getPlayerList().getPlayer(p.player);
        if (player != null) Compat.sendToPlayer(player, new UploadPackets.Result(sink.ticket(), UploadStatus.CANCELLED, ""));
    }

    @Override
    public boolean simulatePick(ServerPlayer player, Path file) {
        if (!(player instanceof FakePlayer)) return false;
        if (file == null) simulated.remove(player.getUUID());
        else simulated.put(player.getUUID(), file);
        return true;
    }

    /**
     * A scenario's player has no game to show a picker in: the server stands
     * in for it, sending the file it was given through the same handlers a
     * real game's packets reach (docs/SCENARIO_EXCEPTIONS.md).
     */
    private void simulate(ServerPlayer player, int ticket, Path file) {
        byte[] bytes;
        try {
            bytes = Files.readAllBytes(file);
        } catch (IOException e) {
            handleCancel(player, new UploadPackets.Cancel(ticket, UploadStatus.FAILED));
            return;
        }
        handleBegin(player, new UploadPackets.Begin(ticket, file.getFileName().toString(), bytes.length));
        for (int off = 0; off < bytes.length && pending.containsKey(ticket); off += UploadPackets.CHUNK) {
            int n = Math.min(UploadPackets.CHUNK, bytes.length - off);
            byte[] chunk = new byte[n];
            System.arraycopy(bytes, off, chunk, 0, n);
            handleChunk(player, new UploadPackets.Chunk(ticket, chunk));
        }
        if (pending.containsKey(ticket)) handleEnd(player, new UploadPackets.End(ticket));
    }

    /** The upload {@code ticket} if it belongs to {@code player} and is still open, else null. */
    private Pending owned(ServerPlayer player, int ticket) {
        Pending p = pending.get(ticket);
        if (p == null || !p.player.equals(player.getUUID())) return null;
        if (p.sink.closed()) {
            pending.remove(ticket);
            return null;
        }
        p.lastTick[0] = tick;
        return p;
    }

    private void end(ServerPlayer player, Pending p, int status) {
        pending.remove(p.sink.ticket());
        if (status < 0) p.sink.fail(status);
        String path = status == UploadStatus.DONE && p.sink.path() != null ? p.sink.path() : "";
        if (!(player instanceof FakePlayer)) Compat.sendToPlayer(player, new UploadPackets.Result(p.sink.ticket(), status, path));
    }

    void handleBegin(ServerPlayer player, UploadPackets.Begin m) {
        Pending p = owned(player, m.ticket());
        if (p == null) {
            refuse(player, m.ticket());
            return;
        }
        int rc = p.sink.begin(m.name(), m.size());
        if (rc != UploadStatus.OK) {
            end(player, p, rc);
            return;
        }
        p.receiving[0] = true;
        LOGGER.info("[upload] {} is uploading {} ({} bytes) to {}", player.getName().getString(), m.name(), m.size(),
                p.sink.destination());
    }

    void handleChunk(ServerPlayer player, UploadPackets.Chunk m) {
        Pending p = owned(player, m.ticket());
        if (p == null) return; // ended: the Result already told the game to stop
        int rc = p.sink.write(m.data(), 0, m.data().length);
        if (rc != UploadStatus.OK) end(player, p, rc);
    }

    void handleEnd(ServerPlayer player, UploadPackets.End m) {
        Pending p = owned(player, m.ticket());
        if (p == null) {
            refuse(player, m.ticket());
            return;
        }
        int rc = p.sink.finish();
        end(player, p, rc == UploadStatus.OK ? UploadStatus.DONE : rc);
    }

    void handleCancel(ServerPlayer player, UploadPackets.Cancel m) {
        Pending p = owned(player, m.ticket());
        if (p == null) return;
        int status = m.status() < 0 ? m.status() : UploadStatus.CANCELLED;
        end(player, p, status);
    }

    /** A packet for an upload that has ended (or was never this player's): tell the game it's over. */
    private void refuse(ServerPlayer player, int ticket) {
        if (!(player instanceof FakePlayer)) Compat.sendToPlayer(player, new UploadPackets.Result(ticket, UploadStatus.UNKNOWN, ""));
    }

    /** Uploads time out; a player who left loses theirs. */
    void tick(net.minecraft.server.MinecraftServer server) {
        tick++;
        currentServer = server;
        if (pending.isEmpty() || tick % 20 != 0) return;
        for (Iterator<Pending> it = pending.values().iterator(); it.hasNext(); ) {
            Pending p = it.next();
            if (p.sink.closed()) {
                it.remove();
                continue;
            }
            // (A scenario's simulated upload finishes inside start(), so every pending one is a real player's.)
            ServerPlayer player = server.getPlayerList().getPlayer(p.player);
            long idle = tick - p.lastTick[0];
            boolean gone = player == null;
            if (gone || idle > (p.receiving[0] ? IDLE_TIMEOUT_TICKS : PICK_TIMEOUT_TICKS)) {
                it.remove();
                p.sink.fail(gone ? UploadStatus.FAILED : (p.receiving[0] ? UploadStatus.FAILED : UploadStatus.CANCELLED));
                if (player != null) Compat.sendToPlayer(player, new UploadPackets.Result(p.sink.ticket(), UploadStatus.FAILED, ""));
            }
        }
    }


    /** Handler wrappers: payloads arrive on the network thread. */
    static void onBegin(UploadPackets.Begin m, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) INSTANCE.handleBegin(p, m);
        });
    }

    static void onChunk(UploadPackets.Chunk m, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) INSTANCE.handleChunk(p, m);
        });
    }

    static void onEnd(UploadPackets.End m, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) INSTANCE.handleEnd(p, m);
        });
    }

    static void onCancel(UploadPackets.Cancel m, IPayloadContext ctx) {
        ctx.enqueueWork(() -> {
            if (ctx.player() instanceof ServerPlayer p) INSTANCE.handleCancel(p, m);
        });
    }
}
