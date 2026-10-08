package com.example.evanscomputermod.radio.wifi.ap;

//? if <=1.21.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.radio.wifi80211.MacAddress;
import com.example.evanscomputermod.radio.wifi80211.Security;
import com.example.evanscomputermod.radio.wifi80211.ap.ApConfig;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.server.level.ServerPlayer;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.ArrayList;
import java.util.List;

/**
 * Access Point GUI traffic. The client never receives the passphrase: views
 * carry only "has a passphrase". The client sends a new passphrase only when
 * the player typed one; the server checks the player may configure this AP
 * (owner, op or creative) and is near it before applying anything.
 */
public final class ApPackets {
    public static final int MAX_STRING = 1024;
    /** Players must be within this distance (blocks) of the AP to change it. */
    public static final double MAX_DISTANCE = 8;

    private ApPackets() {}

    public static void register(RegisterPayloadHandlersEvent event) {
        var registrar = event.registrar("1");
        registrar.playToClient(Status.TYPE, Status.STREAM_CODEC,
                (p, ctx) -> com.example.evanscomputermod.radio.wifi.ap.client.AccessPointClient.onStatus(p.view()));
        registrar.playToServer(Configure.TYPE, Configure.STREAM_CODEC, ApPackets::handleConfigure);
    }

    // ------------------------------------------------------------ view

    public record ClientRow(String mac, int rssiDbm, int rateKbps, String state, String handshake, String lastError) {}

    /** Everything the settings / status screen shows. */
    public record ApView(BlockPos pos, ApSettings settings, boolean hasPassphrase, String bssid, String portMac,
                         int channelInUse, boolean cabled, boolean radioOn, String owner, List<ClientRow> clients,
                         List<String> events, String message, long radioFrames, long wiredIn, long wiredOut) {}

    public static void writeView(FriendlyByteBuf buf, ApView v) {
        buf.writeBlockPos(v.pos());
        writeSettings(buf, v.settings());
        buf.writeBoolean(v.hasPassphrase());
        buf.writeUtf(v.bssid());
        buf.writeUtf(v.portMac());
        buf.writeVarInt(v.channelInUse());
        buf.writeBoolean(v.cabled());
        buf.writeBoolean(v.radioOn());
        buf.writeUtf(v.owner());
        buf.writeVarInt(v.clients().size());
        for (ClientRow r : v.clients()) {
            buf.writeUtf(r.mac());
            buf.writeVarInt(r.rssiDbm());
            buf.writeVarInt(r.rateKbps());
            buf.writeUtf(r.state());
            buf.writeUtf(r.handshake());
            buf.writeUtf(clip(r.lastError()));
        }
        buf.writeVarInt(v.events().size());
        for (String e : v.events()) buf.writeUtf(clip(e));
        buf.writeUtf(clip(v.message()));
        buf.writeVarLong(v.radioFrames());
        buf.writeVarLong(v.wiredIn());
        buf.writeVarLong(v.wiredOut());
    }

    public static ApView readView(FriendlyByteBuf buf) {
        BlockPos pos = buf.readBlockPos();
        ApSettings s = readSettings(buf);
        boolean hasPass = buf.readBoolean();
        String bssid = buf.readUtf(), port = buf.readUtf();
        int ch = buf.readVarInt();
        boolean cabled = buf.readBoolean(), on = buf.readBoolean();
        String owner = buf.readUtf();
        int n = Math.min(buf.readVarInt(), 256);
        List<ClientRow> rows = new ArrayList<>(n);
        for (int i = 0; i < n; i++)
            rows.add(new ClientRow(buf.readUtf(), buf.readVarInt(), buf.readVarInt(), buf.readUtf(), buf.readUtf(), buf.readUtf(MAX_STRING)));
        int m = Math.min(buf.readVarInt(), 64);
        List<String> events = new ArrayList<>(m);
        for (int i = 0; i < m; i++) events.add(buf.readUtf(MAX_STRING));
        String msg = buf.readUtf(MAX_STRING);
        return new ApView(pos, s, hasPass, bssid, port, ch, cabled, on, owner, rows, events, msg,
                buf.readVarLong(), buf.readVarLong(), buf.readVarLong());
    }

    static void writeSettings(FriendlyByteBuf buf, ApSettings s) {
        buf.writeUtf(s.ssid(), 64);
        buf.writeBoolean(s.hidden());
        buf.writeEnum(s.security());
        buf.writeVarInt(s.channelSetting());
        buf.writeVarInt(s.txPowerDbm());
        buf.writeBoolean(s.clientIsolation());
        buf.writeEnum(s.filterMode());
        buf.writeVarInt(s.filterMacs().size());
        for (MacAddress m : s.filterMacs()) buf.writeLong(m.value());
    }

    static ApSettings readSettings(FriendlyByteBuf buf) {
        String ssid = buf.readUtf(64);
        boolean hidden = buf.readBoolean();
        Security sec = buf.readEnum(Security.class);
        int ch = buf.readVarInt(), tx = buf.readVarInt();
        boolean iso = buf.readBoolean();
        ApConfig.MacFilterMode mode = buf.readEnum(ApConfig.MacFilterMode.class);
        int n = buf.readVarInt();
        if (n < 0 || n > ApSettings.MAX_FILTER_MACS) throw new IllegalArgumentException("too many MAC filter entries");
        List<MacAddress> macs = new ArrayList<>(n);
        for (int i = 0; i < n; i++) macs.add(new MacAddress(buf.readLong() & 0xFFFF_FFFF_FFFFL));
        return new ApSettings(ssid, hidden, sec, ch, tx, iso, mode, macs);
    }

    private static String clip(String s) {
        if (s == null) return "";
        return s.length() > 200 ? s.substring(0, 200) : s;
    }

    // ------------------------------------------------------------ server -> client

    /** A fresh view for an open AP screen (sent every half second, and after each change). */
    public record Status(ApView view) implements CustomPacketPayload {
        public static final Type<Status> TYPE = new Type<>(EvansComputerMod.id("access_point_status"));
        public static final StreamCodec<FriendlyByteBuf, Status> STREAM_CODEC =
                StreamCodec.of((buf, p) -> writeView(buf, p.view), buf -> new Status(readView(buf)));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    // ------------------------------------------------------------ client -> server

    public enum Action { APPLY, KICK }

    /**
     * Apply settings (with {@code passphrase} empty = keep the stored one), or
     * kick the client {@code kickMac}.
     */
    public record Configure(BlockPos pos, Action action, ApSettings settings, String passphrase, long kickMac)
            implements CustomPacketPayload {
        public static final Type<Configure> TYPE = new Type<>(EvansComputerMod.id("access_point_configure"));
        public static final StreamCodec<FriendlyByteBuf, Configure> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> {
                    buf.writeBlockPos(p.pos);
                    buf.writeEnum(p.action);
                    writeSettings(buf, p.settings);
                    buf.writeUtf(p.passphrase, 64);
                    buf.writeLong(p.kickMac);
                },
                buf -> new Configure(buf.readBlockPos(), buf.readEnum(Action.class), readSettings(buf), buf.readUtf(64), buf.readLong()));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    private static void handleConfigure(Configure p, IPayloadContext ctx) {
        if (!(ctx.player() instanceof ServerPlayer player)) return;
        String reply = configure(player, p);
        if (player.level().getBlockEntity(p.pos()) instanceof AccessPointBlockEntity be)
            PacketDistributor.sendToPlayer(player, new Status(be.view(reply)));
    }

    /**
     * Server side of the Configure packet (also called directly by tests).
     * Returns the message to show: the outcome, or why it was refused.
     */
    public static String configure(ServerPlayer player, Configure p) {
        if (!(player.level().getBlockEntity(p.pos()) instanceof AccessPointBlockEntity be)) return "No access point there";
        if (player.distanceToSqr(p.pos().getCenter()) > MAX_DISTANCE * MAX_DISTANCE) return "Too far from the access point";
        if (!be.canConfigure(player)) return "Only " + (be.ownerName() == null ? "the owner" : be.ownerName()) + " can change this access point";
        return switch (p.action()) {
            case APPLY -> {
                String err = be.applySettings(p.settings(), p.passphrase());
                yield err == null ? "Settings applied" : "Not applied: " + err;
            }
            case KICK -> be.deauthenticate(new MacAddress(p.kickMac() & 0xFFFF_FFFF_FFFFL)) ? "Client deauthenticated" : "No such client";
        };
    }
}
//?}
