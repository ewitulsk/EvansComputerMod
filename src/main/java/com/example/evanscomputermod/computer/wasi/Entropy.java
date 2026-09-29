package com.example.evanscomputermod.computer.wasi;

import java.security.SecureRandom;

/**
 * Entropy for guests (kernel getrandom and WASI random_get). SSH keys, key
 * exchange and TCP sequence numbers come from it, so it must be a CSPRNG,
 * not java.util.Random.
 */
public final class Entropy {
    private static final SecureRandom RNG = new SecureRandom();

    public static void fill(byte[] bytes) {
        RNG.nextBytes(bytes);
    }

    private Entropy() {}
}
