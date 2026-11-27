package com.example.evanscomputermod.computer.repo;

import java.security.KeyFactory;
import java.security.KeyPairGenerator;
import java.security.SecureRandom;
import java.security.Signature;
import java.security.interfaces.EdECPublicKey;
import java.security.spec.EdECPoint;
import java.security.spec.EdECPrivateKeySpec;
import java.security.spec.NamedParameterSpec;

/** ed25519 from a 32-byte seed, as the programs' {@code ecm_pkg::sign} and the secret store use it. */
public final class Ed25519 {
    private Ed25519() {}

    /** The 32-byte public key of a seed. */
    public static byte[] publicKey(byte[] seed) {
        try {
            KeyPairGenerator g = KeyPairGenerator.getInstance("Ed25519");
            g.initialize(NamedParameterSpec.ED25519, new FixedRandom(seed));
            EdECPoint p = ((EdECPublicKey) g.generateKeyPair().getPublic()).getPoint();
            // RFC 8032: y little-endian, the top bit of the last byte is x's parity.
            byte[] be = p.getY().toByteArray();
            byte[] out = new byte[32];
            for (int i = 0; i < 32 && i < be.length; i++) out[i] = be[be.length - 1 - i];
            if (p.isXOdd()) out[31] |= (byte) 0x80;
            return out;
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** The 64-byte signature over {@code message}. */
    public static byte[] sign(byte[] seed, byte[] message) {
        try {
            var key = KeyFactory.getInstance("Ed25519").generatePrivate(new EdECPrivateKeySpec(NamedParameterSpec.ED25519, seed));
            Signature s = Signature.getInstance("Ed25519");
            s.initSign(key);
            s.update(message);
            return s.sign();
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    public static String hex(byte[] b) {
        StringBuilder sb = new StringBuilder(b.length * 2);
        for (byte x : b) sb.append(Character.forDigit((x >> 4) & 15, 16)).append(Character.forDigit(x & 15, 16));
        return sb.toString();
    }

    /** Hands the key generator exactly the seed (an ed25519 private key is the 32 random bytes it draws). */
    private static final class FixedRandom extends SecureRandom {
        private final byte[] seed;

        FixedRandom(byte[] seed) {
            this.seed = seed.clone();
        }

        @Override
        public void nextBytes(byte[] bytes) {
            if (bytes.length != seed.length) throw new IllegalStateException("ed25519 wants " + bytes.length + " bytes");
            System.arraycopy(seed, 0, bytes, 0, bytes.length);
        }
    }
}
