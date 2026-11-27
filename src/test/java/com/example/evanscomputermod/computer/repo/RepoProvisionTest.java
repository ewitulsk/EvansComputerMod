package com.example.evanscomputermod.computer.repo;

import static org.junit.jupiter.api.Assertions.assertEquals;
import static org.junit.jupiter.api.Assertions.assertNull;
import static org.junit.jupiter.api.Assertions.assertTrue;

import java.util.List;
import org.junit.jupiter.api.Test;

/** The official repository's lines in /etc/pkg/repos for each server setting (docs/REPO.md). */
class RepoProvisionTest {
    static final String KEY = "ab".repeat(32), URL = "https://repo.example";
    static final String HEADER = "# pkg repositories: <name> <url> [<pinned ed25519 key>]\n";
    static final String VILLAGE = "Tech-Village-Apps http://100.68.0.21:8080/ " + "cd".repeat(32);

    @Test
    void eachAccessModeGivesItsLines() {
        assertEquals(List.of("official host://repo/r/stable/ " + KEY, "community host://repo/r/community/ " + KEY),
                RepoProvision.wanted("open", URL, KEY, true, false));
        assertEquals(List.of("official host://repo/r/stable/ " + KEY), RepoProvision.wanted("open", URL, KEY, false, false));
        assertEquals(List.of("official " + URL + "/r/stable/ " + KEY, "community " + URL + "/r/community/ " + KEY),
                RepoProvision.wanted("internet", URL, KEY, true, false));
        // datacenter: the server's link for repository servers only.
        assertEquals(List.of(), RepoProvision.wanted("datacenter", URL, KEY, true, false));
        assertEquals("official host://repo/r/stable/ " + KEY, RepoProvision.wanted("datacenter", URL, KEY, true, true).get(0));
        assertEquals(List.of(), RepoProvision.wanted("off", URL, KEY, true, true));
        // No key configured (or a bad one): nothing, whatever the mode.
        assertEquals(List.of(), RepoProvision.wanted("open", URL, "", true, false));
        assertEquals(List.of(), RepoProvision.wanted("open", URL, "xyz", true, false));
    }

    @Test
    void aVillageComputerGetsTheDataCentersCopyInDatacenterMode() {
        String copy = RepoProvision.villageCopy(HEADER + VILLAGE + "\n");
        assertEquals("http://100.68.0.21:8081/", copy);
        assertNull(RepoProvision.villageCopy(HEADER + "mine http://10.0.0.5:8080/\n"));
        assertEquals(List.of("official http://100.68.0.21:8081/ " + KEY), RepoProvision.wanted("datacenter", URL, KEY, true, false, copy));
        // The package server itself (a repository server) uses the server's link, and other modes ignore the copy.
        assertEquals("official host://repo/r/stable/ " + KEY, RepoProvision.wanted("datacenter", URL, KEY, true, true, copy).get(0));
        assertEquals("official host://repo/r/stable/ " + KEY, RepoProvision.wanted("open", URL, KEY, true, false, copy).get(0));
    }

    @Test
    void onlyItsOwnLinesAreRewritten() {
        String mine = "mine http://10.0.0.5:8080/";
        String own = "official http://10.0.0.9:8080/ " + KEY; // a player's own repository called official
        String current = HEADER + VILLAGE + "\n" + mine + "\n" + own + "\n";
        String open = RepoProvision.merge(current, RepoProvision.wanted("open", URL, KEY, true, false));
        assertEquals(HEADER + "official host://repo/r/stable/ " + KEY + "\ncommunity host://repo/r/community/ " + KEY + "\n" + VILLAGE + "\n" + mine + "\n" + own + "\n", open);
        // Switching mode replaces the managed lines and keeps the rest; the same settings change nothing.
        assertNull(RepoProvision.merge(open, RepoProvision.wanted("open", URL, KEY, true, false)));
        String dc = RepoProvision.merge(open, RepoProvision.wanted("datacenter", URL, KEY, true, false, RepoProvision.villageCopy(open)));
        assertEquals(HEADER + "official http://100.68.0.21:8081/ " + KEY + "\n" + VILLAGE + "\n" + mine + "\n" + own + "\n", dc);
        String off = RepoProvision.merge(dc, RepoProvision.wanted("off", URL, KEY, true, false));
        assertEquals(current, off);
        // A village computer on a server with no repository configured keeps exactly its generated file.
        assertNull(RepoProvision.merge(HEADER + VILLAGE + "\n", List.of()));
        assertTrue(RepoProvision.managed("official http://100.68.0.21:8081/ " + KEY));
    }
}
