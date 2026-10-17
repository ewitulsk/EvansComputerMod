package com.example.evanscomputermod.computer;

import java.io.IOException;
import java.io.InputStream;
import java.nio.charset.StandardCharsets;
import java.security.MessageDigest;
import java.util.LinkedHashMap;
import java.util.List;
import java.util.Map;

/**
 * The Tech Village accounts (docs/AUTH.md, docs/TECH_VILLAGE_NETWORK.md "Accounts and music"):
 * the chat village's Data Center computer runs {@code authd} with a realm key derived from the
 * world seed (like {@link VillageRepo}'s repository key), and the ring's {@code discordd} and
 * {@code musicd} trust only that key. Every village computer and Phone Store phone ships with
 * the realm pinned in Discord's and Music's own logins, so the village realm is trusted from
 * the first launch. Plain Java: the formats are {@code ecm_auth}'s (the fingerprint is the first
 * 16 bytes of SHA-256 of the public key, as hex).
 */
public record VillageAuth(String publicKey, byte[] secret) {
    /** The realm's name (shown on the sign-in screen). */
    public static final String NAME = "Tech Village";
    public static final int AUTH_PORT = 7600, MUSIC_PORT = 7800;
    /** What the ring's music server is called. */
    public static final String MUSIC_NAME = "Tech Village Radio";
    /** Where authd keeps the realm key on the chat server (hex seed). */
    public static final String KEY_FILE = "etc/auth/realm.key";
    /** The apps whose logins get the realm pinned. */
    public static final List<String> APPS = List.of("dev.ecm.discord", "dev.ecm.music");
    /** The songs the music server starts with (docs/MUSIC_CREDITS.md). */
    static final String MUSIC_RESOURCES = "/evanscomputermod/village/music/";
    static final List<String> SONGS = List.of("01-redstone-sunrise.mp3", "02-village-fiber.mp3", "03-data-center-nights.mp3");

    /** This world's realm. */
    public static VillageAuth of(long seed) {
        byte[] secret = secretOf(seed);
        return new VillageAuth(VillageRepo.publicKeyHex(secret), secret);
    }

    /** The 32-byte ed25519 seed: HMAC-SHA256(world seed, "ECM-TECHVILLAGE-AUTH"). Server-side only. */
    public static byte[] secretOf(long seed) {
        try {
            var mac = javax.crypto.Mac.getInstance("HmacSHA256");
            byte[] k = new byte[8];
            for (int i = 0; i < 8; i++) k[i] = (byte) (seed >>> (56 - 8 * i));
            mac.init(new javax.crypto.spec.SecretKeySpec(k, "HmacSHA256"));
            return mac.doFinal("ECM-TECHVILLAGE-AUTH".getBytes(StandardCharsets.US_ASCII));
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    /** {@code ecm_auth::fingerprint}: lower-case hex of the first 16 bytes of SHA-256(public key). */
    public static String fingerprint(String publicKeyHex) {
        try {
            byte[] h = MessageDigest.getInstance("SHA-256").digest(VillageRepo.unhex(publicKeyHex));
            return CellPlan.hex(java.util.Arrays.copyOf(h, 16)).toLowerCase(java.util.Locale.ROOT);
        } catch (Exception e) {
            throw new IllegalStateException(e);
        }
    }

    public String fingerprint() {
        return fingerprint(publicKey);
    }

    /** The ring's Discord backend port (WorldNetwork.CHAT_PORT; this class is version-neutral). */
    static final int CHAT_PORT = 7777;

    /** The chat server's address in village {@code chat} (WorldNetwork.chatAddress). */
    static String chatAddress(int chat) {
        return "100." + (64 + chat) + ".0.20";
    }

    /** The realm's address: authd on the chat server. */
    public static String endpoint(int chat) {
        return chatAddress(chat) + ":" + AUTH_PORT;
    }

    /** The chat server's services: authd first, then the backends that trust it. */
    public String services(int chat) {
        String at = endpoint(chat);
        return "authd " + AUTH_PORT + " --name \"" + NAME + "\" --key-file /" + KEY_FILE + " &\n"
                + "discordd " + CHAT_PORT + " --name \"" + VillageDiscovery.DISCORD_NAME + "\" --auth " + at + " --auth-key " + fingerprint() + " &\n"
                + "musicd " + MUSIC_PORT + " --name \"" + MUSIC_NAME + "\" --library /srv/music --auth " + at + " --auth-key " + fingerprint() + " &\n";
    }

    /**
     * The pins file each app keeps ({@code ecm_auth} store: {@code <fp> <host:port> <name>}): the
     * village realm, trusted from the first launch.
     */
    public String pins(int chat) {
        return fingerprint() + " " + endpoint(chat) + " " + NAME + "\n";
    }

    /** Every computer's pinned realm for Discord and Music. */
    public Map<String, String> pinFiles(int chat) {
        Map<String, String> files = new LinkedHashMap<>();
        for (String app : APPS) files.put("var/apps/" + app + "/accounts/pins", pins(chat));
        return files;
    }

    /** The chat server's realm key file (hex seed: authd --key-file). */
    public String keyFile() {
        return CellPlan.hex(secret).toLowerCase(java.util.Locale.ROOT) + "\n";
    }

    /** The music server's starting library: {@code srv/music/<song>} -> bytes (empty in a build without them). */
    public static Map<String, byte[]> songs() {
        Map<String, byte[]> out = new LinkedHashMap<>();
        for (String s : SONGS) {
            try (InputStream in = VillageAuth.class.getResourceAsStream(MUSIC_RESOURCES + s)) {
                if (in != null) out.put("srv/music/" + s, in.readAllBytes());
            } catch (IOException ignored) {
                // a jar without the songs: the server starts empty
            }
        }
        return out;
    }
}
