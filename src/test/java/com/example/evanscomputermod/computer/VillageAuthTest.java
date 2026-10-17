package com.example.evanscomputermod.computer;

import org.junit.jupiter.api.Test;

import java.util.Map;

import static org.junit.jupiter.api.Assertions.*;

/** The Tech Village realm: the same key and fingerprint as ecm_auth (its village_realm_vector_matches_java test). */
class VillageAuthTest {

    @Test
    void realmKeyAndFingerprintMatchEcmAuth() {
        VillageAuth a = VillageAuth.of(123456L);
        assertEquals("ae21bd8070b17d3f542f24343a794cc659b13c6095ff822301c70f008f7f29cb", a.keyFile().trim());
        assertEquals("bea5d9599b7cec6ade0b4c4b8c03d3bb", a.fingerprint());
        assertNotEquals(VillageRepo.of(123456L, 1).publicKey(), a.publicKey(), "the realm key is not the repository key");
    }

    @Test
    void theChatServerRunsTheRealmAndBothBackendsTrustIt() {
        VillageAuth a = VillageAuth.of(42L);
        String services = a.services(3);
        assertTrue(services.startsWith("authd 7600 --name \"Tech Village\" --key-file /etc/auth/realm.key &"), services);
        assertTrue(services.contains("discordd 7777 --name \"Tech Village\" --auth 100.67.0.20:7600 --auth-key " + a.fingerprint()), services);
        assertTrue(services.contains("musicd 7800 --name \"Tech Village Radio\" --library /srv/music --auth 100.67.0.20:7600 --auth-key "
                + a.fingerprint()), services);
        // Every computer, house PCs too, has the realm pinned in Discord's and Music's logins.
        Map<String, String> pins = a.pinFiles(3);
        for (String app : VillageAuth.APPS) {
            assertEquals(a.fingerprint() + " 100.67.0.20:7600 Tech Village\n", pins.get("var/apps/" + app + "/accounts/pins"));
        }
        assertEquals(a.fingerprint() + " 100.67.0.20:7600 Tech Village\n", pins.get("home/.chat/accounts/pins"), "the console's chat too");
        // The ISP answers for auth and music ring-wide.
        String disco = VillageDiscovery.ispConf("100.67.0.20", 7777, null, true);
        assertTrue(disco.contains("record auth 100.67.0.20 7600 Tech Village") && disco.contains("record music 100.67.0.20 7800 Tech Village Radio"), disco);
        assertFalse(VillageDiscovery.ispConf("100.67.0.20", 7777, null, false).contains("record auth"), "without accounts, as before");
    }

    @Test
    void theVillageSongsShipWithTheMod() {
        Map<String, byte[]> songs = VillageAuth.songs();
        assertEquals(3, songs.size(), songs.keySet().toString());
        for (byte[] b : songs.values()) assertTrue(b.length > 100_000 && b[0] == 'I' && b[1] == 'D' && b[2] == '3', "an ID3-tagged MP3");
    }
}
