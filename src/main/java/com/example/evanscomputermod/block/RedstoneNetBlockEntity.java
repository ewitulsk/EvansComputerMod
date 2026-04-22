package com.example.evanscomputermod.block;

import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.computer.CableNetworkManager;
import com.example.evanscomputermod.computer.NetworkHub;
import com.example.evanscomputermod.computer.netdev.EcmProto;
import com.example.evanscomputermod.computer.netdev.NetDevice;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.core.HolderLookup;
import net.minecraft.core.UUIDUtil;
import net.minecraft.nbt.CompoundTag;
//? if >=26.1 {
import net.minecraft.world.level.storage.ValueInput;
import net.minecraft.world.level.storage.ValueOutput;
//?}
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.level.Level;
import net.minecraft.world.level.block.entity.BlockEntity;
import net.minecraft.world.level.block.entity.BlockEntityTicker;
import net.minecraft.world.level.block.state.BlockState;

import java.util.UUID;

/**
 * Block entity for the {@link RedstoneNetBlock}. Extends {@link NetDevice}-style
 * behaviour via composition (BlockEntity cannot multi-inherit) and translates
 * ECM protocol messages into redstone output on the 6 faces of the block.
 *
 * <p>Packet types handled (see {@link EcmProto}):
 * <ul>
 *   <li>{@code TYPE_PING} → {@code TYPE_PONG}</li>
 *   <li>{@code TYPE_DESCRIBE} → {@code TYPE_DESCRIBE_REPLY} with kind=REDSTONE</li>
 *   <li>{@code TYPE_REDSTONE_SET} [side:u8][level:u8] → apply and reply {@code TYPE_REDSTONE_ACK}</li>
 *   <li>{@code TYPE_REDSTONE_GET} → {@code TYPE_REDSTONE_REPORT} with 6 level bytes</li>
 * </ul>
 */
public class RedstoneNetBlockEntity extends BlockEntity {

    /** Identity persists across unload/load so the MAC is stable. */
    private UUID deviceId;

    /** Redstone output levels per Direction ordinal (0-15). */
    private final int[] redstoneOutput = new int[6];

    private NetDevice device;
    private boolean registeredWithCableMgr;

    // Persisted config (defaults chosen to be obviously distinct from the
    // computers' default subnet — operators will want to reconfigure).
    private int configuredIp;        // 0 = unconfigured
    private int configuredPrefix;    // 0 = unconfigured
    private int configuredPort = EcmProto.DEFAULT_PORT;

    public RedstoneNetBlockEntity(BlockPos pos, BlockState state) {
        super(ModBlockEntities.REDSTONE_NET_BLOCK_ENTITY.get(), pos, state);
        this.deviceId = UUID.randomUUID();
    }

    public UUID getDeviceId() { return deviceId; }

    public void setDeviceId(UUID id) { this.deviceId = id; }

    public int getRedstoneOutput(int side) {
        return (side >= 0 && side < 6) ? redstoneOutput[side] : 0;
    }

    public int getConfiguredIp() { return configuredIp; }
    public int getConfiguredPrefix() { return configuredPrefix; }
    public int getConfiguredPort() { return configuredPort; }

    /** Applies a new static IP and persists it. Called from the /ecmnetdev command. */
    public void configure(int ip, int prefixLen) {
        this.configuredIp = ip;
        this.configuredPrefix = prefixLen;
        if (device != null) {
            device.setIp(ip, prefixLen);
        }
        setChanged();
    }

    public void configurePort(int port) {
        this.configuredPort = port;
        if (device != null) {
            device.setAppPort(port);
        }
        setChanged();
    }

    // ==================== Ticker ====================

    public static <T extends BlockEntity> BlockEntityTicker<T> createTicker(Level level) {
        if (level.isClientSide()) return null;
        return (lvl, pos, state, be) -> {
            if (be instanceof RedstoneNetBlockEntity rbe) rbe.serverTick();
        };
    }

    private void serverTick() {
        // Lazy attachment: NetworkHub may not yet be initialised at chunk-load
        // time. Once it is, register our NIC. Also re-register with the cable
        // manager if needed (handles hot-reload during world already loaded).
        ensureAttached();
    }

    private void ensureAttached() {
        if (device != null && device.isAttached()) return;
        NetworkHub hub = NetworkHub.getInstance();
        if (hub == null) return;

        if (device == null) {
            byte[] mac = NetworkHub.deriveMac(deviceId);
            device = new DeviceImpl(mac);
            if (configuredPrefix > 0) {
                device.setIp(configuredIp, configuredPrefix);
            }
            device.setAppPort(configuredPort);
        }
        device.attach();

        if (!registeredWithCableMgr && level instanceof ServerLevel sl) {
            CableNetworkManager cableMgr = CableNetworkManager.getInstance();
            if (cableMgr != null) {
                // Exit position: the one block adjacent that a cable should
                // touch. We pick DOWN as a convention — cables can reach us
                // from any side anyway because cables look at all 6 neighbors.
                // The exit is just the BFS seed.
                BlockPos exit = worldPosition.below();
                cableMgr.registerDevice(worldPosition, sl.dimension(), device.getMac(), exit);
                registeredWithCableMgr = true;
            }
        }
    }

    @Override
    public void setRemoved() {
        try {
            if (device != null) {
                device.detach();
                if (registeredWithCableMgr) {
                    CableNetworkManager cableMgr = CableNetworkManager.getInstance();
                    if (cableMgr != null) cableMgr.unregisterDevice(device.getMac());
                    registeredWithCableMgr = false;
                }
            }
        } finally {
            super.setRemoved();
        }
    }

    // ==================== NetDevice implementation ====================

    private final class DeviceImpl extends NetDevice {
        DeviceImpl(byte[] mac) { super(mac); }

        @Override
        protected int getDeviceKind() { return EcmProto.KIND_REDSTONE; }

        @Override
        protected void onAppPacket(int srcIp, int srcPort, byte[] payload, int off, int len) {
            if (!EcmProto.hasMagic(payload, off, len)) return;
            int type = EcmProto.getType(payload, off);
            int seq = EcmProto.getSeq(payload, off);
            int bodyOff = off + EcmProto.HEADER_LEN;
            int bodyLen = len - EcmProto.HEADER_LEN;

            switch (type) {
                case EcmProto.TYPE_PING -> {
                    byte[] reply = new byte[EcmProto.HEADER_LEN];
                    EcmProto.writeHeader(reply, 0, EcmProto.TYPE_PONG, EcmProto.KIND_REDSTONE, seq);
                    sendUdpReply(srcIp, srcPort, reply);
                }
                case EcmProto.TYPE_DESCRIBE -> {
                    // "redstone" (9 bytes, room to grow later)
                    byte[] name = "redstone".getBytes();
                    byte[] reply = new byte[EcmProto.HEADER_LEN + 1 + name.length];
                    int p = EcmProto.writeHeader(reply, 0, EcmProto.TYPE_DESCRIBE_REPLY, EcmProto.KIND_REDSTONE, seq);
                    reply[p++] = (byte) name.length;
                    System.arraycopy(name, 0, reply, p, name.length);
                    sendUdpReply(srcIp, srcPort, reply);
                }
                case EcmProto.TYPE_REDSTONE_SET -> {
                    if (bodyLen < 2) { sendError(srcIp, srcPort, seq); return; }
                    int side = payload[bodyOff] & 0xff;
                    int lvl = payload[bodyOff + 1] & 0xff;
                    if (side >= 6 || lvl > 15) { sendError(srcIp, srcPort, seq); return; }
                    applyRedstoneSet(side, lvl);
                    byte[] reply = new byte[EcmProto.HEADER_LEN + 2];
                    int p = EcmProto.writeHeader(reply, 0, EcmProto.TYPE_REDSTONE_ACK, EcmProto.KIND_REDSTONE, seq);
                    reply[p] = (byte) side;
                    reply[p + 1] = (byte) lvl;
                    sendUdpReply(srcIp, srcPort, reply);
                }
                case EcmProto.TYPE_REDSTONE_GET -> {
                    byte[] reply = new byte[EcmProto.HEADER_LEN + 6];
                    int p = EcmProto.writeHeader(reply, 0, EcmProto.TYPE_REDSTONE_REPORT, EcmProto.KIND_REDSTONE, seq);
                    for (int i = 0; i < 6; i++) reply[p + i] = (byte) redstoneOutput[i];
                    sendUdpReply(srcIp, srcPort, reply);
                }
                default -> sendError(srcIp, srcPort, seq);
            }
        }

        private void sendError(int srcIp, int srcPort, int seq) {
            byte[] reply = new byte[EcmProto.HEADER_LEN];
            EcmProto.writeHeader(reply, 0, EcmProto.TYPE_ERROR, EcmProto.KIND_REDSTONE, seq);
            sendUdpReply(srcIp, srcPort, reply);
        }
    }

    /**
     * Applies a redstone level to the requested side. Must run on the server
     * main thread because it mutates world state and triggers neighbor updates.
     */
    private void applyRedstoneSet(int side, int level) {
        if (this.level == null || !(this.level instanceof ServerLevel serverLevel)) return;
        MinecraftServer server = serverLevel.getServer();
        server.execute(() -> {
            if (isRemoved()) return;
            if (redstoneOutput[side] == level) return;
            redstoneOutput[side] = level;
            setChanged();
            // Force a block update so neighboring redstone re-reads our signal.
            BlockState state = getBlockState();
            serverLevel.setBlock(worldPosition, state, 3);
            // Explicit neighbor update on the affected face.
            Direction dir = Direction.values()[side];
            BlockPos neighbor = worldPosition.relative(dir);
            serverLevel.updateNeighborsAt(neighbor, state.getBlock());
        });
    }

    // ==================== NBT ====================

    //? if >=26.1 {
    @Override
    protected void saveAdditional(ValueOutput output) {
        super.saveAdditional(output);
        output.store("deviceId", UUIDUtil.CODEC, deviceId);
        output.putIntArray("redstoneOutput", redstoneOutput);
        output.putInt("ip", configuredIp);
        output.putInt("prefix", configuredPrefix);
        output.putInt("port", configuredPort);
    }

    @Override
    protected void loadAdditional(ValueInput input) {
        super.loadAdditional(input);
        input.read("deviceId", UUIDUtil.CODEC).ifPresent(id -> deviceId = id);
        input.getIntArray("redstoneOutput").ifPresent(saved -> {
            System.arraycopy(saved, 0, redstoneOutput, 0, Math.min(saved.length, 6));
        });
        configuredIp = input.read("ip", com.mojang.serialization.Codec.INT).orElse(0);
        configuredPrefix = input.read("prefix", com.mojang.serialization.Codec.INT).orElse(0);
        configuredPort = input.read("port", com.mojang.serialization.Codec.INT).orElse(EcmProto.DEFAULT_PORT);
    }
    //?} else {
    /*@Override
    protected void saveAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.saveAdditional(tag, registries);
        UUIDUtil.CODEC.encodeStart(net.minecraft.nbt.NbtOps.INSTANCE, deviceId)
                .result().ifPresent(t -> tag.put("deviceId", t));
        tag.putIntArray("redstoneOutput", redstoneOutput);
        tag.putInt("ip", configuredIp);
        tag.putInt("prefix", configuredPrefix);
        tag.putInt("port", configuredPort);
    }

    @Override
    protected void loadAdditional(CompoundTag tag, HolderLookup.Provider registries) {
        super.loadAdditional(tag, registries);
        if (tag.contains("deviceId")) {
            UUIDUtil.CODEC.parse(net.minecraft.nbt.NbtOps.INSTANCE, tag.get("deviceId"))
                    .result().ifPresent(id -> deviceId = id);
        }
        if (tag.contains("redstoneOutput")) {
            int[] saved = tag.getIntArray("redstoneOutput");
            System.arraycopy(saved, 0, redstoneOutput, 0, Math.min(saved.length, 6));
        }
        configuredIp = tag.contains("ip") ? tag.getInt("ip") : 0;
        configuredPrefix = tag.contains("prefix") ? tag.getInt("prefix") : 0;
        configuredPort = tag.contains("port") ? tag.getInt("port") : EcmProto.DEFAULT_PORT;
    }*/
    //?}
}
