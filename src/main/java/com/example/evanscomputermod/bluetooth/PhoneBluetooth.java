package com.example.evanscomputermod.bluetooth;

//? if <=1.21.1 {
import com.example.evanscomputermod.radio.api.Channel;
import com.example.evanscomputermod.radio.api.Pose;
import com.example.evanscomputermod.radio.medium.RadioMediumHooks;
import com.example.evanscomputermod.voice.AirTrack;
import com.example.evanscomputermod.voice.SoundField;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.nbt.ListTag;
import net.minecraft.nbt.Tag;

import java.security.SecureRandom;
import java.util.ArrayList;
import java.util.HexFormat;
import java.util.List;
import java.util.UUID;

/**
 * The ePhone's Bluetooth radio: a {@link PhoneLink} on two {@link BtEndpoint}s (one
 * always listening on the advertising channel, one on the link's data channel), at
 * the phone's antenna. It also keeps the headset's microphone as an air track the
 * phone's {@code /dev/mic} reads while the audio is routed to the headset (the
 * phone's "headphones layer", docs/BLUETOOTH.md "Audio route").
 */
public final class PhoneBluetooth {

    private static final SecureRandom RANDOM = new SecureRandom();

    private final BtEndpoint adv, data;
    private final PhoneLink link;
    /** What the headset's microphone heard, on the sound field's clock. */
    private final AirTrack headsetMic = new AirTrack();
    private volatile long headsetMicAtNanos;

    public PhoneBluetooth(UUID phoneId, String name) {
        long addr = BtAddress.of(phoneId);
        adv = new BtEndpoint(new UUID(phoneId.getMostSignificantBits() ^ 0x0B1E_0000L, phoneId.getLeastSignificantBits()));
        data = new BtEndpoint(new UUID(phoneId.getMostSignificantBits() ^ 0x0B1E_0001L, phoneId.getLeastSignificantBits()));
        adv.tune(BtAddress.ADV);
        link = new PhoneLink(addr, name, new PhoneLink.Radio() {
            @Override public void send(BtFrame frame, Channel ch) {
                if (ch == null) adv.transmit(frame, BtAddress.ADV);
                else data.transmit(frame, ch);
            }

            @Override public void tuneData(Channel ch) {
                data.tune(ch);
            }
        }, () -> {
            byte[] k = new byte[16];
            RANDOM.nextBytes(k);
            return k;
        });
        link.setMicSink((pcm, rate) -> {
            SoundField f = SoundField.get();
            long now = f.nanos();
            headsetMicAtNanos = now;
            headsetMic.append(pcm, 0, pcm.length, rate, f.nowAir(), now, 1f);
        });
    }

    public PhoneLink link() {
        return link;
    }

    /** Where the connected headset's buttons go (a double-tap plays or pauses the phone's media). */
    public void setButtonSink(PhoneLink.ButtonSink sink) {
        link.setButtonSink(sink);
    }

    /** One server tick, with the phone's antenna at {@code pose}; {@code micWanted}: a program reads /dev/mic. */
    public void tick(Pose pose, boolean micWanted) {
        var medium = RadioMediumHooks.medium();
        adv.sync(link.powered() ? medium : null);
        data.sync(link.powered() ? medium : null);
        adv.moveTo(pose);
        data.moveTo(pose);
        long now = System.currentTimeMillis();
        for (BtEndpoint.Heard h; (h = adv.poll()) != null; ) link.receive(h.frame(), h.rssiDbm(), now);
        for (BtEndpoint.Heard h; (h = data.poll()) != null; ) link.receive(h.frame(), h.rssiDbm(), now);
        link.tick(now, micWanted);
    }

    /** The phone stopped (off, put away): the radio goes quiet. */
    public void stop() {
        adv.close();
        data.close();
    }

    /** The phone's audio goes to the headset. */
    public boolean audioRouted() {
        return link.audioRouted();
    }

    /** The phone's microphone is the headset's (while the audio is routed there). */
    public boolean micRouted() {
        return link.audioRouted();
    }

    /** Add what the headset's microphone heard to {@code out} (the phone's /dev/mic while routed). */
    public void renderMic(long start, int n, int rate, float[] out) {
        headsetMic.addTo(out, start, n, rate, 1f);
    }

    /** The connected headset's address, or -1. */
    public long connected() {
        return link.connected();
    }

    // ------------------------------------------------------------------ saved on the phone item

    public CompoundTag save() {
        CompoundTag t = new CompoundTag();
        t.putBoolean("powered", link.powered());
        ListTag list = new ListTag();
        for (PhoneLink.Paired p : link.paired()) {
            CompoundTag d = new CompoundTag();
            d.putString("addr", BtAddress.format(p.addr()));
            d.putString("name", p.name());
            d.putString("key", HexFormat.of().formatHex(p.key()));
            list.add(d);
        }
        t.put("paired", list);
        return t;
    }

    public void load(CompoundTag t) {
        if (t == null || t.isEmpty()) return;
        link.setPowered(!t.contains("powered") || t.getBoolean("powered"));
        List<PhoneLink.Paired> out = new ArrayList<>();
        for (Tag x : t.getList("paired", Tag.TAG_COMPOUND)) {
            CompoundTag d = (CompoundTag) x;
            long a = BtAddress.parse(d.getString("addr"));
            if (a < 0) continue;
            try {
                out.add(new PhoneLink.Paired(a, d.getString("name"), HexFormat.of().parseHex(d.getString("key"))));
            } catch (IllegalArgumentException ignored) {
            }
        }
        link.restore(out);
        link.takeChanged();
    }
}
//?}
