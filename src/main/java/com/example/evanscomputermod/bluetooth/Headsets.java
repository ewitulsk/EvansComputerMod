package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.speaker.ImaAdpcm;
import com.example.evanscomputermod.voice.SoundField;
import com.example.evanscomputermod.voice.VoiceServer;
import net.minecraft.server.MinecraftServer;
import net.minecraft.server.level.ServerPlayer;
import net.minecraft.world.entity.EquipmentSlot;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.neoforged.neoforge.common.util.FakePlayer;
import net.neoforged.neoforge.event.tick.ServerTickEvent;
import net.neoforged.neoforge.network.PacketDistributor;

import java.util.HashSet;
import java.util.Map;
import java.util.Set;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;
import java.util.function.Consumer;

/**
 * The AirPods in the world, as radios (docs/BLUETOOTH.md): every server tick each
 * player's worn AirPods, and the AirPods in a case whose lid is open, get a
 * {@link HeadsetLink} on a {@link BtEndpoint} at the player's head. What the phone
 * sends is played in the wearer's ears only ({@link BluetoothPackets.EarAudio});
 * when the phone wants the microphone, the AirPods hear the sound field at the
 * wearer's mouth and send it back. Battery: worn and connected they run down;
 * in the case they charge.
 */
public final class Headsets {

    /** Battery used per second while worn and connected, and charged per second in the case, percent. */
    public static final double DRAIN_PER_S = 100.0 / (3 * 3600), CHARGE_PER_S = 100.0 / 600;
    /** Microphone rate the AirPods send at. */
    public static final int MIC_RATE = 16_000;

    private static final class Headset {
        final UUID id;
        final BtEndpoint ep;
        final HeadsetLink link;
        ServerPlayer wearer;
        boolean worn;
        Consumer<AirPodsData.Pods> write = p -> {};
        AirPodsData.Pods pods;
        long micNext = Long.MIN_VALUE;
        int earSeq;
        final ImaAdpcm ear = new ImaAdpcm();
        long earSamples;
        double battery;

        Headset(AirPodsData.Pods pods, String owner) {
            id = pods.id();
            this.pods = pods;
            battery = pods.battery();
            ep = new BtEndpoint(new UUID(id.getMostSignificantBits() ^ 0xA1D0_0000L, id.getLeastSignificantBits()));
            String name = owner.isEmpty() ? "AirPods" : owner + "'s AirPods";
            link = new HeadsetLink(pods.address(), name, new HeadsetLink.Radio() {
                @Override public void send(BtFrame f, Channel data) { ep.transmit(f, data == null ? BtAddress.ADV : data); }
                @Override public void tune(Channel c) { ep.tune(c); }
            }, new HeadsetLink.Owner() {
                @Override public void paired(long phone, byte[] key, String phoneName) {
                    Headset.this.pods = Headset.this.pods.paired(phone, key, phoneName);
                    write.accept(Headset.this.pods.withBattery(battery));
                }

                @Override public void play(short[] pcm, int rate) {
                    playInEars(Headset.this, pcm, rate);
                }
            });
            link.restore(pods.pairedTo(), pods.key());
        }
    }

    private static final Map<UUID, Headset> HEADSETS = new ConcurrentHashMap<>();

    private Headsets() {}

    private static void playInEars(Headset h, short[] pcm, int rate) {
        ServerPlayer p = h.wearer;
        if (p == null || !h.worn) return;
        h.earSamples += pcm.length;
        if (p instanceof FakePlayer) return;
        for (int off = 0; off < pcm.length; off += 2600) {
            int n = Math.min(2600, pcm.length - off);
            PacketDistributor.sendToPlayer(p, new BluetoothPackets.EarAudio(h.earSeq++, rate, n, h.ear.encode(pcm, off, n)));
        }
    }

    public static void onServerTick(ServerTickEvent.Post e) {
        tick(e.getServer());
    }

    static void tick(MinecraftServer server) {
        long now = System.currentTimeMillis();
        Set<UUID> live = new HashSet<>();
        for (ServerPlayer p : PhoneServer.players(server)) {
            if (p.isRemoved() || p.isSpectator()) continue;
            ItemStack head = p.getItemBySlot(EquipmentSlot.HEAD);
            if (head.getItem() instanceof AirPodsItem) {
                AirPodsData.Pods pods = AirPodsData.pods(head);
                live.add(pods.id());
                run(p, pods, true, false, x -> AirPodsData.setPods(head, x), now, server.getTickCount());
            }
            Inventory inv = p.getInventory();
            for (int i = 0; i < inv.getContainerSize(); i++) {
                ItemStack s = inv.getItem(i);
                if (!(s.getItem() instanceof AirPodsCaseItem)) continue;
                AirPodsData.Pods inside = AirPodsData.inCase(s);
                if (inside == null || live.contains(inside.id())) continue;
                boolean open = AirPodsCaseItem.lidOpen(s, p.level());
                charge(s, inside, server.getTickCount());
                if (!open) continue;
                live.add(inside.id());
                run(p, AirPodsData.inCase(s), false, true, x -> AirPodsData.setInCase(s, x), now, server.getTickCount());
            }
        }
        HEADSETS.entrySet().removeIf(en -> {
            if (live.contains(en.getKey())) return false;
            en.getValue().ep.close();
            return true;
        });
    }

    /** AirPods in a case charge (written every second). */
    private static void charge(ItemStack caseStack, AirPodsData.Pods inside, int tick) {
        if (inside.battery() >= 100 || tick % 20 != 0) return;
        AirPodsData.setInCase(caseStack, inside.withBattery(inside.battery() + CHARGE_PER_S));
    }

    private static void run(ServerPlayer p, AirPodsData.Pods pods, boolean worn, boolean pairable,
                            Consumer<AirPodsData.Pods> write, long now, int tick) {
        Headset h = HEADSETS.computeIfAbsent(pods.id(), k -> new Headset(pods, p.getGameProfile().getName()));
        h.wearer = p;
        h.worn = worn;
        h.write = write;
        if (!worn) h.battery = pods.battery();
        var eye = p.getEyePosition();
        h.ep.moveTo(Pose.at(p.level().dimension().location().toString(), eye.x, eye.y - 0.1, eye.z));
        h.ep.sync(RadioMediumHooks.medium());
        for (BtEndpoint.Heard x; (x = h.ep.poll()) != null; ) h.link.receive(x.frame(), now);
        h.link.setState(pairable, worn, (int) Math.ceil(h.battery), 255);
        h.link.tick(now);
        if (worn && h.link.connected()) {
            h.battery = Math.max(0, h.battery - DRAIN_PER_S / 20);
            if (tick % 20 == 0) write.accept(h.pods.withBattery(h.battery));
        }
        if (h.link.micWanted()) sendMic(h, p);
        else h.micNext = Long.MIN_VALUE;
    }

    /** What the AirPods' microphones hear at the wearer's mouth since the last tick, to the phone. */
    private static void sendMic(Headset h, ServerPlayer p) {
        SoundField field = SoundField.get();
        long end = field.sampleAt(MIC_RATE);
        if (h.micNext == Long.MIN_VALUE || end - h.micNext > MIC_RATE / 2) h.micNext = end - MIC_RATE / 20;
        int n = (int) (end - h.micNext);
        if (n <= 0) return;
        float[] buf = new float[n];
        VoiceServer.Spot m = VoiceServer.mouth(p);
        field.render(m.dimension(), m.x(), m.y(), m.z(), h.micNext, n, MIC_RATE, buf);
        h.micNext = end;
        short[] pcm = new short[n];
        for (int i = 0; i < n; i++) pcm[i] = (short) Math.round(Math.max(-1, Math.min(1, buf[i])) * 32767);
        h.link.sendMic(pcm, MIC_RATE);
    }

    // ------------------------------------------------------------------ queries

    /** Who wears the AirPods with this address (the phone's microphone is theirs while routed), or null. */
    public static ServerPlayer wearerOf(long addr) {
        for (Headset h : HEADSETS.values()) if (h.worn && h.link.addr == addr) return h.wearer;
        return null;
    }

    /** Samples of phone audio played in this player's ears so far (tests, debug). */
    public static long earSamples(UUID player) {
        long n = 0;
        for (Headset h : HEADSETS.values()) if (h.wearer != null && h.wearer.getUUID().equals(player)) n += h.earSamples;
        return n;
    }

    /** The AirPods with this id are connected to a phone (tests, debug). */
    public static boolean connected(UUID pods) {
        Headset h = HEADSETS.get(pods);
        return h != null && h.link.connected();
    }
}
//?}
