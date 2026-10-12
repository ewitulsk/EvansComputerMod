package com.example.evanscomputermod.card;

import static org.junit.jupiter.api.Assertions.*;

import com.mojang.serialization.JsonOps;
import io.netty.buffer.ByteBuf;
import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import net.minecraft.core.RegistryAccess;
import net.minecraft.core.component.DataComponentType;
import net.minecraft.network.RegistryFriendlyByteBuf;
import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.util.HexFormat;

/**
 * The SIM smart card: the key never in client-synced data, the respond() derivation
 * (vectors computed independently with Python's hmac module, documented in
 * docs/gui/CELLULAR_V2.md), and write-once.
 */
public class SimCardTest {
    static final String KEY = "00112233445566778899aabbccddeeff";
    static final HexFormat HEX = HexFormat.of();

    // ------------------------------------------------------------ the key stays on the server

    private static byte[] networkBytes(DataComponentType<SimSecret> type, SimSecret value) {
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), RegistryAccess.EMPTY);
        type.streamCodec().encode(buf, value);
        return ByteBufUtil.getBytes(buf);
    }

    @Test
    void keyIsNotInTheClientSyncedComponent() {
        DataComponentType<SimSecret> type = SimSecret.componentType();
        SimSecret secret = new SimSecret(KEY);
        byte[] wire = networkBytes(type, secret);
        String asText = new String(wire, StandardCharsets.ISO_8859_1);
        assertFalse(asText.contains(KEY), "hex key on the wire");
        assertFalse(ByteBufUtil.indexOf(Unpooled.wrappedBuffer(HEX.parseHex(KEY)), Unpooled.wrappedBuffer(wire)) >= 0, "raw key on the wire");
        assertEquals(0, wire.length, "the secret component sends nothing");
        // What a client decodes: the placeholder, which can't answer a challenge.
        RegistryFriendlyByteBuf in = new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(wire), RegistryAccess.EMPTY);
        SimSecret client = type.streamCodec().decode(in);
        assertSame(SimSecret.HIDDEN, client);
        assertFalse(client.known());
        assertThrows(IllegalStateException.class, client::key);
        // The persistent codec (the save) does keep it.
        String saved = type.codec().encodeStart(JsonOps.INSTANCE, secret).getOrThrow().toString();
        assertTrue(saved.contains(KEY), saved);
        assertEquals(secret, type.codec().parse(JsonOps.INSTANCE, type.codec().encodeStart(JsonOps.INSTANCE, secret).getOrThrow()).getOrThrow());
        assertFalse(secret.toString().contains(KEY), "toString must not print the key");
    }

    @Test
    void identityIsSyncedAndCarriesNoKey() {
        DataComponentType<SimIdentity> type = SimIdentity.componentType();
        SimIdentity id = new SimIdentity("cell-red", 310_150_000_000_001L);
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), RegistryAccess.EMPTY);
        type.streamCodec().encode(buf, id);
        assertEquals(id, type.streamCodec().decode(buf));
        assertEquals("310150000000001", id.imsiText());
    }

    // ------------------------------------------------------------ respond() derivation

    @Test
    void respondDerivationVectors() {
        byte[] k = HEX.parseHex(KEY);
        SimAuth.Response r = SimAuth.respond(k, HEX.parseHex("000102030405060708090a0b0c0d0e0f"));
        assertEquals("4c455ba887affefa1448bea727baa584", r.resHex());
        assertEquals("3a2295de88d4ea04764491a508856d342f71ed47a7415c5bffeabd934308713a", r.sessionKeyHex());
        r = SimAuth.respond(k, HEX.parseHex("ffffffffffffffffffffffffffffffff"));
        assertEquals("3e316dff329533c535f8b5106208b75c", r.resHex());
        assertEquals("4531753cba8d31b05e36c2c9fd04a80f1baac7d541c8a580b21b2cd17f9cf09e", r.sessionKeyHex());
        r = SimAuth.respond(new byte[16], new byte[16]);
        assertEquals("7fbf4baffaedda01afa5c9856ba7a9f4", r.resHex());
        assertEquals("027c68857ae22afa9558fa8d747477918e9a90cb23146cb02617eeafabb9dd23", r.sessionKeyHex());
        assertTrue(SimAuth.verify(k, new byte[16], SimAuth.respond(k, new byte[16]).res()));
        assertFalse(SimAuth.verify(new byte[16], new byte[16], SimAuth.respond(k, new byte[16]).res()));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.respond(k, new byte[8]));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.respond(new byte[8], new byte[16]));
    }

    // ------------------------------------------------------------ write once

    static final class MemoryCard implements SmartSim.Storage {
        SimIdentity id;
        SimSecret secret;
        int count = 1;
        int writes;

        @Override public SimIdentity identity() { return id; }
        @Override public SimSecret secret() { return secret; }
        @Override public int count() { return count; }
        @Override public void store(SimIdentity identity, SimSecret s) { id = identity; secret = s; writes++; }
    }

    @Test
    void writeOnlyWhenBlank() throws Exception {
        MemoryCard m = new MemoryCard();
        SmartSim card = new SmartSim(m);
        assertTrue(card.isBlank());
        assertNull(card.identity());
        assertThrows(SmartSim.CardException.class, () -> card.respond(new byte[16]), "a blank card answers nothing");

        card.write(310_150_000_000_001L, "cell-red", HEX.parseHex(KEY));
        assertFalse(card.isBlank());
        assertEquals(new SimIdentity("cell-red", 310_150_000_000_001L), card.identity());
        assertEquals("4c455ba887affefa1448bea727baa584",
                card.respond(HEX.parseHex("000102030405060708090a0b0c0d0e0f")).resHex());

        SmartSim.CardException e = assertThrows(SmartSim.CardException.class,
                () -> card.write(310_150_000_000_009L, "cell-blue", new byte[16]));
        assertTrue(e.getMessage().contains("not blank"), e.getMessage());
        assertEquals(1, m.writes);
        assertEquals("cell-red", card.identity().carrier(), "a rejected write changes nothing");

        // A client's copy (key hidden) is written but can't answer.
        m.secret = SimSecret.HIDDEN;
        assertThrows(SmartSim.CardException.class, () -> card.respond(new byte[16]));

        // Stacks of blank cards are written one at a time; bad input is refused.
        MemoryCard stack = new MemoryCard();
        stack.count = 4;
        assertThrows(SmartSim.CardException.class, () -> new SmartSim(stack).write(1_000_000L, "x", new byte[16]));
        MemoryCard one = new MemoryCard();
        assertThrows(SmartSim.CardException.class, () -> new SmartSim(one).write(1_000_000L, "Bad Carrier!", new byte[16]));
        assertThrows(SmartSim.CardException.class, () -> new SmartSim(one).write(1_000_000L, "ok", new byte[15]));
        assertEquals(0, one.writes);
    }

    @Test
    void parsing() {
        assertEquals(310150000000001L, SimAuth.parseImsi("310150000000001"));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.parseImsi("12345"));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.parseImsi("1234567890123456"));
        assertEquals("cell-red", SimAuth.parseCarrier("Cell-Red"));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.parseKey("0011"));
        assertThrows(IllegalArgumentException.class, () -> SimAuth.parseKey("zz112233445566778899aabbccddeeff"));
        assertArrayEquals(HEX.parseHex(KEY), SimAuth.parseKey(KEY.toUpperCase()));
    }
}
