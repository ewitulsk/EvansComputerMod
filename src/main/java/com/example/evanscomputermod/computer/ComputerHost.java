package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.*;
import com.example.evanscomputermod.block.TerminalBlockEntity;

import net.minecraft.server.MinecraftServer;

import java.util.*;

/** Server-owned computer lifetime. Blocks attach to hosts; unloading is not a reboot. */
public final class ComputerHost implements IComputerHost {
    private static final Map<MinecraftServer, Map<UUID, ComputerHost>> SERVERS =
            new WeakHashMap<>();
    private final MinecraftServer server;
    private final UUID id;
    private volatile IComputerHost attachment;
    private final TerminalDisplay headlessDisplay = new TerminalDisplay();
    private volatile boolean booting;

    public synchronized boolean beginBoot() {
        if (booting || instance != null) return false;
        booting = true;
        return true;
    }

    public void endBoot() {
        booting = false;
    }

    public boolean isBooting() {
        return booting;
    }

    public IComputerHost attachment() {
        return attachment;
    }

    public TerminalDisplay headlessDisplay() {
        return headlessDisplay;
    }

    private ComputerInstance instance;
    private TerminalBlockEntity.TransferBundle bundle;
    private boolean infrastructure;

    private ComputerHost(MinecraftServer server, UUID id) {
        this.server = server;
        this.id = id;
    }

    public static synchronized ComputerHost get(MinecraftServer server, UUID id) {
        return SERVERS.computeIfAbsent(server, k -> new HashMap<>())
                .computeIfAbsent(id, k -> new ComputerHost(server, id));
    }

    public static synchronized List<ComputerHost> list(MinecraftServer server) {
        return List.copyOf(SERVERS.getOrDefault(server, Map.of()).values());
    }

    public static synchronized void shutdown(MinecraftServer server) {
        Map<UUID, ComputerHost> hosts = SERVERS.remove(server);
        if (hosts != null)
            for (ComputerHost h : hosts.values())
                if (h.instance != null) {
                    h.instance.close();
                    h.instance = null;
                }
    }

    public void attach(IComputerHost block) {
        attachment = block;
    }

    public void track(ComputerInstance computer) {
        instance = computer;
        computer.setHost(this);
    }

    public ComputerInstance instance() {
        return instance;
    }

    public boolean isHeadless() {
        return instance != null && attachment == null;
    }

    public void setInfrastructure(boolean value) {
        infrastructure = value;
    }

    public boolean isInfrastructure() {
        return infrastructure;
    }

    public boolean canDetach() {
        if (infrastructure) return true;
        long count = list(server).stream().filter(h -> h.isHeadless() && !h.infrastructure).count();
        return count < Math.max(0, Integer.getInteger("evanscomputermod.headlessCap", 64));
    }

    public void detach(TerminalBlockEntity.TransferBundle transfer) {
        bundle = transfer;
        instance = transfer.computer;
        instance.setHost(this);
        attachment = null;
        //? if <=1.21.1 {
        if (!infrastructure) {
            var d = WorldNetwork.get(server.overworld());
            d.alwaysOn.put(id, instance.getNetworkMacs().length);
            d.setDirty();
        }
        //?}
    }

    public TerminalBlockEntity.TransferBundle takeBundle(IComputerHost block) {
        attachment = block;
        var result = bundle;
        bundle = null;
        return result;
    }

    public void forget() {
        instance = null;
        bundle = null;
        attachment = null;
        //? if <=1.21.1 {
        if (server != null) {
            var d = WorldNetwork.get(server.overworld());
            if (d.alwaysOn.remove(id) != null) d.setDirty();
        }
        //?}
    }

    public UUID getComputerId() {
        return id;
    }

    public MinecraftServer getServer() {
        return server;
    }

    public void markDirty() {
        IComputerHost h = attachment;
        if (h != null) h.markDirty();
    }

    public void syncToClients() {
        IComputerHost h = attachment;
        if (h != null) h.syncToClients();
    }

    public void forceNextKeyframe() {
        IComputerHost h = attachment;
        if (h != null) h.forceNextKeyframe();
    }

    public IFramebufferDisplay getFramebufferDisplay() {
        IComputerHost h = attachment;
        return h == null ? headlessDisplay : h.getFramebufferDisplay();
    }

    public IRedstoneProvider getRedstoneProvider() {
        IComputerHost h = attachment;
        return h == null ? null : h.getRedstoneProvider();
    }

    public IWorldAccess getWorldAccess() {
        IComputerHost h = attachment;
        return h == null ? null : h.getWorldAccess();
    }

    public IVisualProgramming getVisualProgramming() {
        IComputerHost h = attachment;
        return h == null ? null : h.getVisualProgramming();
    }

    public com.example.evanscomputermod.computer.peripheral.PeripheralHub getPeripheralHub() {
        IComputerHost h = attachment;
        return h == null ? null : h.getPeripheralHub();
    }
}
