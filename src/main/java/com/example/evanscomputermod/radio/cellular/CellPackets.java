package com.example.evanscomputermod.radio.cellular;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.EvansComputerMod;
import net.minecraft.core.BlockPos;
import net.minecraft.network.FriendlyByteBuf;
import net.minecraft.network.codec.StreamCodec;
import net.minecraft.network.protocol.common.custom.CustomPacketPayload;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.player.Player;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.network.PacketDistributor;
import net.neoforged.neoforge.network.event.RegisterPayloadHandlersEvent;
import net.neoforged.neoforge.network.handling.IPayloadContext;

import java.util.ArrayList;
import java.util.List;

/**
 * Cell tower screen traffic: a view of the tower and its carrier's SIMs to the
 * client, and the three carrier actions back (apply settings, issue a SIM, set
 * a SIM's data limit). The server checks the player may run the tower / carrier
 * and is near the tower before doing anything. SIM keys never leave the server
 * except on the issued SIM item itself.
 */
public final class CellPackets {
    public static final double MAX_DISTANCE = 8;
    static final int MAX_ROWS = 128;

    private CellPackets() {}

    public static void register(RegisterPayloadHandlersEvent event) {
        var registrar = event.registrar("1");
        registrar.playToClient(Status.TYPE, Status.STREAM_CODEC,
                (p, ctx) -> com.example.evanscomputermod.radio.cellular.client.CellClient.onStatus(p.view()));
        registrar.playToServer(Action.TYPE, Action.STREAM_CODEC, CellPackets::handleAction);
    }

    // ------------------------------------------------------------ view

    /** One technology of the tower. */
    public record TechRow(String name, int channel, double freqMhz, int antennas, int heightM, boolean onAir, int devices) {}

    /** One SIM of the carrier: what the screen's list shows. {@code attached} is "" or the technology it is on. */
    public record SimRow(long imsi, String label, String attached, double rssiDbm, int cqi, long usedBytes, long quotaBytes,
                         boolean revoked) {}

    public record TowerView(BlockPos pos, String carrier, String owner, String carrierOwner, int lteChannel, int mmChannel,
                            boolean cabled, List<TechRow> techs, List<SimRow> sims, String message) {}

    public static TowerView view(CellTowerBlockEntity be, String message) {
        List<TechRow> techs = new ArrayList<>();
        for (PhyProfile t : PhyProfile.values()) {
            TowerLink l = be.link(t);
            CellTowerBlockEntity.Mounts m = be.mounts(t);
            techs.add(new TechRow(t.displayName, be.channel(t), t.downlink(be.channel(t)).centerHz() / 1e6, m.count(), m.mastHeight(),
                    l != null && l.onAir(), l == null ? 0 : l.deviceCount()));
        }
        List<SimRow> sims = new ArrayList<>();
        String carrierOwner = "";
        if (be.getLevel() != null && be.getLevel().getServer() != null && be.carrier() != null) {
            CarrierRegistry.Carrier c = CarrierRegistry.get(be.getLevel().getServer()).carrier(be.carrier());
            if (c != null) {
                carrierOwner = c.ownerName == null ? "" : c.ownerName;
                for (CarrierRegistry.Subscriber s : c.subscribers()) {
                    String on = "";
                    double rssi = Double.NaN;
                    int cqi = 0;
                    for (PhyProfile t : PhyProfile.values()) {
                        TowerLink l = be.link(t);
                        if (l == null) continue;
                        for (TowerLink.DeviceStatus d : l.devices()) {
                            if (d.imsi() != s.imsi) continue;
                            on = t.displayName;
                            rssi = d.ulRssiDbm();
                            cqi = d.dlCqi();
                        }
                    }
                    if (sims.size() < MAX_ROWS) sims.add(new SimRow(s.imsi, s.label, on, rssi, cqi, s.usedBytes, s.quotaBytes, s.revoked));
                }
            }
        }
        String msg = message != null ? message : be.lastMessage() == null ? "" : be.lastMessage();
        return new TowerView(be.getBlockPos(), be.carrier() == null ? "" : be.carrier(), be.ownerName() == null ? "" : be.ownerName(),
                carrierOwner, be.channel(PhyProfile.LTE), be.channel(PhyProfile.MMWAVE), be.cabled(), techs, sims, msg);
    }

    public static void writeView(FriendlyByteBuf buf, TowerView v) {
        buf.writeBlockPos(v.pos());
        buf.writeUtf(v.carrier(), 64);
        buf.writeUtf(v.owner(), 64);
        buf.writeUtf(v.carrierOwner(), 64);
        buf.writeVarInt(v.lteChannel());
        buf.writeVarInt(v.mmChannel());
        buf.writeBoolean(v.cabled());
        buf.writeVarInt(v.techs().size());
        for (TechRow t : v.techs()) {
            buf.writeUtf(t.name(), 32);
            buf.writeVarInt(t.channel());
            buf.writeDouble(t.freqMhz());
            buf.writeVarInt(t.antennas());
            buf.writeVarInt(t.heightM());
            buf.writeBoolean(t.onAir());
            buf.writeVarInt(t.devices());
        }
        buf.writeVarInt(v.sims().size());
        for (SimRow s : v.sims()) {
            buf.writeLong(s.imsi());
            buf.writeUtf(s.label(), 64);
            buf.writeUtf(s.attached(), 32);
            buf.writeDouble(s.rssiDbm());
            buf.writeVarInt(s.cqi());
            buf.writeVarLong(s.usedBytes());
            buf.writeLong(s.quotaBytes());
            buf.writeBoolean(s.revoked());
        }
        buf.writeUtf(clip(v.message()), 256);
    }

    public static TowerView readView(FriendlyByteBuf buf) {
        BlockPos pos = buf.readBlockPos();
        String carrier = buf.readUtf(64), owner = buf.readUtf(64), carrierOwner = buf.readUtf(64);
        int lte = buf.readVarInt(), mm = buf.readVarInt();
        boolean cabled = buf.readBoolean();
        int n = Math.min(buf.readVarInt(), 8);
        List<TechRow> techs = new ArrayList<>(n);
        for (int i = 0; i < n; i++)
            techs.add(new TechRow(buf.readUtf(32), buf.readVarInt(), buf.readDouble(), buf.readVarInt(), buf.readVarInt(), buf.readBoolean(),
                    buf.readVarInt()));
        int m = Math.min(buf.readVarInt(), MAX_ROWS);
        List<SimRow> sims = new ArrayList<>(m);
        for (int i = 0; i < m; i++)
            sims.add(new SimRow(buf.readLong(), buf.readUtf(64), buf.readUtf(32), buf.readDouble(), buf.readVarInt(), buf.readVarLong(),
                    buf.readLong(), buf.readBoolean()));
        return new TowerView(pos, carrier, owner, carrierOwner, lte, mm, cabled, techs, sims, buf.readUtf(256));
    }

    private static String clip(String s) {
        if (s == null) return "";
        return s.length() > 200 ? s.substring(0, 200) : s;
    }

    // ------------------------------------------------------------ server -> client

    public record Status(TowerView view) implements CustomPacketPayload {
        public static final Type<Status> TYPE = new Type<>(EvansComputerMod.id("cell_tower_status"));
        public static final StreamCodec<FriendlyByteBuf, Status> STREAM_CODEC =
                StreamCodec.of((buf, p) -> writeView(buf, p.view), buf -> new Status(readView(buf)));

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    // ------------------------------------------------------------ client -> server

    public enum Kind { APPLY, ISSUE_SIM, SET_QUOTA }

    /**
     * A carrier action from the screen. APPLY uses {@code carrier}, {@code lteChannel},
     * {@code mmChannel}; ISSUE_SIM uses {@code text} as the SIM's label (consuming one
     * blank SIM card from the player's inventory unless they're in creative); SET_QUOTA
     * uses {@code imsi} and {@code quotaBytes} (negative = no limit).
     */
    public record Action(BlockPos pos, Kind kind, String carrier, int lteChannel, int mmChannel, String text, long imsi, long quotaBytes)
            implements CustomPacketPayload {
        public static final Type<Action> TYPE = new Type<>(EvansComputerMod.id("cell_tower_action"));
        public static final StreamCodec<FriendlyByteBuf, Action> STREAM_CODEC = StreamCodec.of(
                (buf, p) -> {
                    buf.writeBlockPos(p.pos);
                    buf.writeEnum(p.kind);
                    buf.writeUtf(p.carrier, 64);
                    buf.writeVarInt(p.lteChannel);
                    buf.writeVarInt(p.mmChannel);
                    buf.writeUtf(p.text, 64);
                    buf.writeLong(p.imsi);
                    buf.writeLong(p.quotaBytes);
                },
                buf -> new Action(buf.readBlockPos(), buf.readEnum(Kind.class), buf.readUtf(64), buf.readVarInt(), buf.readVarInt(),
                        buf.readUtf(64), buf.readLong(), buf.readLong()));

        public static Action apply(BlockPos pos, String carrier, int lte, int mm) {
            return new Action(pos, Kind.APPLY, carrier, lte, mm, "", 0, 0);
        }

        public static Action issue(BlockPos pos, String label) {
            return new Action(pos, Kind.ISSUE_SIM, "", 1, 1, label == null ? "" : label, 0, 0);
        }

        public static Action quota(BlockPos pos, long imsi, long bytes) {
            return new Action(pos, Kind.SET_QUOTA, "", 1, 1, "", imsi, bytes);
        }

        @Override
        public Type<? extends CustomPacketPayload> type() {
            return TYPE;
        }
    }

    private static void handleAction(Action p, IPayloadContext ctx) {
        if (!(ctx.player() instanceof ServerPlayer player)) return;
        String reply = act(player, p);
        if (player.level().getBlockEntity(p.pos()) instanceof CellTowerBlockEntity be)
            PacketDistributor.sendToPlayer(player, new Status(view(be, reply)));
    }

    /** Server side of an {@link Action} (also called directly by scenarios). Returns the message to show. */
    public static String act(Player player, Action p) {
        if (!(player.level().getBlockEntity(p.pos()) instanceof CellTowerBlockEntity be)) return "No cell tower there";
        if (player.distanceToSqr(p.pos().getCenter()) > MAX_DISTANCE * MAX_DISTANCE) return "Too far from the tower";
        return switch (p.kind()) {
            case APPLY -> {
                if (!be.canConfigure(player)) yield "Only " + (be.ownerName() == null ? "the owner" : be.ownerName()) + " can change this tower";
                String err = be.applySettings(player, p.carrier(), p.lteChannel(), p.mmChannel());
                yield err == null ? "Settings applied" : "Not applied: " + err;
            }
            case ISSUE_SIM -> {
                ItemStack blank = ItemStack.EMPTY;
                if (!player.getAbilities().instabuild) {
                    var inv = player.getInventory();
                    for (int i = 0; i < inv.getContainerSize() && blank.isEmpty(); i++) {
                        ItemStack s = inv.getItem(i);
                        if (s.getItem() instanceof SimCardItem && !SimCardItem.isIssued(s)) blank = s;
                    }
                    if (blank.isEmpty()) yield "You need a blank SIM card";
                } else {
                    blank = new ItemStack(CellularContent.SIM_CARD.get());
                }
                SimCard sim = be.issueSim(player, p.text());
                if (sim == null) yield be.lastMessage() == null ? "Can't issue a SIM" : be.lastMessage();
                if (!player.getAbilities().instabuild) blank.shrink(1);
                ItemStack out = SimCardItem.issued(sim);
                if (!player.getInventory().add(out)) player.drop(out, false);
                yield "Issued " + sim.carrier() + " SIM " + sim.imsiText();
            }
            case SET_QUOTA -> {
                String err = be.setQuota(player, p.imsi(), p.quotaBytes());
                yield err == null ? be.lastMessage() : "Not set: " + err;
            }
        };
    }
}
//?}
