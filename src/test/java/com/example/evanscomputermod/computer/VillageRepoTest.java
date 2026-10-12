package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.nio.charset.StandardCharsets;
import java.util.Arrays;
import java.util.regex.Pattern;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The Tech Village package repository ({@link VillageRepo}, docs/TECH_VILLAGE_NETWORK.md "Apps"):
 * the mod's ed25519 agrees with {@code pkg}'s ({@code ecm_pkg::sign}, the vector in
 * {@code rust/wasm-programs/pkg/src/sign.rs}), the key is a deterministic function of the seed,
 * the repository line is what {@code pkg repo add ... --key} writes, and the package server's
 * files (the Chat package the mod was built with, its signature, publisher.pub) verify.
 */
class VillageRepoTest {
    @Test
    void ed25519AgreesWithPkg() {
        byte[] seed = new byte[32];
        Arrays.fill(seed, (byte) 7);
        String pk = VillageRepo.publicKeyHex(seed);
        assertEquals("ea4a6c63e29c520abef5507b132ec5f9954776aebebe7b92421eea691446d22c", pk);
        byte[] msg = VillageRepo.signedMessage("dev.ecm.chat", "1.0.0", "ab".repeat(32));
        String sig = VillageRepo.sign(seed, msg);
        assertEquals("c8e1a12f3b1e34bbb3c9766ff9b69276653f98f518ba56e48474e81cf64c39c7"
                + "a31b22de211eb83f3063e18970a615c1ae6c94352b3e7b650d2eee83660e4101", sig);
        assertTrue(VillageRepo.verify(pk, msg, sig));
        // Controls: another message, another key, a damaged signature.
        assertFalse(VillageRepo.verify(pk, VillageRepo.signedMessage("dev.ecm.chat", "1.0.1", "ab".repeat(32)), sig));
        Arrays.fill(seed, (byte) 8);
        assertFalse(VillageRepo.verify(VillageRepo.publicKeyHex(seed), msg, sig));
        assertFalse(VillageRepo.verify(pk, msg, "00" + sig.substring(2)));
    }

    @Test
    void theKeyComesFromTheSeed() {
        VillageRepo a = VillageRepo.of(73198425L, 4), again = VillageRepo.of(73198425L, 4), other = VillageRepo.of(73198426L, 4);
        assertEquals(a.publicKey(), again.publicKey());
        assertArrayEquals(a.secret(), again.secret());
        assertNotEquals(a.publicKey(), other.publicKey());
        assertEquals(64, a.publicKey().length());
        assertEquals(a.publicKey(), VillageRepo.publicKeyHex(VillageRepo.secretOf(73198425L)));
        assertEquals("100.68.0.21", a.address());
        assertEquals("http://100.68.0.21:8080/", a.url());
        // /etc/pkg/repos as ecm_pkg::format_repos writes it: one line, a valid name, the URL, the pinned key.
        String repos = a.reposFile();
        assertTrue(repos.startsWith("# pkg repositories: <name> <url> [<pinned ed25519 key>]\n"), repos);
        assertTrue(Pattern.compile("(?m)^Tech-Village-Apps http://100\\.68\\.0\\.21:8080/ [0-9a-f]{64}$").matcher(repos).find(), repos);
        assertTrue(VillageRepo.NAME.matches("[A-Za-z0-9_-]{1,32}"));
        assertEquals("pkg serve /srv/repo 8080 &", VillageRepo.serveCommand());
    }

    @Test
    void theServerHoldsOnlyChatSignedWithTheWorldsKey() {
        var packages = VillageRepo.packages();
        assertEquals(1, packages.size(), "the mod's repository (wasm-bin/repo, staged by scripts/stage-wasm.sh): " + packages);
        var chat = packages.get(0);
        assertEquals("dev.ecm.chat", chat.id());
        assertEquals(chat.id() + "-" + chat.version() + ".ecmpkg", chat.file());
        VillageRepo repo = VillageRepo.of(42L, 7);
        var files = repo.serverFiles();
        assertArrayEquals(chat.bytes(), files.get("srv/repo/" + chat.file()));
        String sig = new String(files.get("srv/repo/" + chat.file() + ".sig"), StandardCharsets.US_ASCII);
        assertTrue(VillageRepo.verify(repo.publicKey(), VillageRepo.signedMessage(chat.id(), chat.version(), chat.sha256()), sig));
        assertEquals(repo.publicKey(), new String(files.get("srv/repo/publisher.pub"), StandardCharsets.US_ASCII));
        assertEquals(repo.publicKey(), VillageRepo.publicKeyHex(VillageRepo.unhex(new String(files.get("etc/pkg/signing.key"), StandardCharsets.US_ASCII))));
        // Control: another world's key does not verify this world's signature.
        assertFalse(VillageRepo.verify(VillageRepo.of(43L, 7).publicKey(), VillageRepo.signedMessage(chat.id(), chat.version(), chat.sha256()), sig));
        assertEquals(4, files.size(), files.keySet().toString());
    }
}
