package com.example.evanscomputermod.computer.apps;

import com.example.evanscomputermod.api.video.VideoBackend;
import com.example.evanscomputermod.api.video.VideoBackends;
import com.example.evanscomputermod.api.video.VideoSource;
import org.junit.jupiter.api.AfterEach;
import org.junit.jupiter.api.Test;

import java.nio.file.Files;
import java.nio.file.Path;

import static org.junit.jupiter.api.Assertions.*;

/**
 * The Video Player is offered only where video can be decoded: with a video
 * backend (the evanscomputermod-ffmpeg sidecar) {@code SystemApps} installs
 * it; without one it isn't installed, and a copy installed while the sidecar
 * was there is removed. Control: a folder the player changed is left alone.
 * The main mod's own test classpath has no backend.
 */
class SystemAppsVideoTest {
    private Path root;

    @AfterEach
    void cleanUp() throws Exception {
        VideoBackends.reset();
        if (root != null) {
            try (var walk = Files.walk(root)) {
                walk.sorted(java.util.Comparator.reverseOrder()).forEach(p -> p.toFile().delete());
            }
        }
    }

    /** A backend that claims to decode (its open is never called here). */
    private static final VideoBackend FAKE = new VideoBackend() {
        @Override public String name() { return "fake"; }
        @Override public int priority() { return 1; }
        @Override public int apiVersion() { return CURRENT_API_VERSION; }
        @Override public boolean isAvailable() { return true; }
        @Override public VideoSource open(Path file, int w, int h, int format) { throw new UnsupportedOperationException(); }
    };

    @Test
    void theVideoPlayerComesAndGoesWithTheVideoBackend() throws Exception {
        root = Files.createTempDirectory("ecm-sysapps");
        Path bin = Files.createDirectories(root.resolve("bin"));
        Path disk = Files.createDirectories(root.resolve("disk"));
        Files.write(bin.resolve("videoplayer.wasm"), new byte[]{0, 'a', 's', 'm', 1, 0, 0, 0});
        PermissionStore store = new PermissionStore(disk);
        Path app = disk.resolve("apps/dev.ecm.video");

        VideoBackends.reset();
        assertFalse(VideoBackends.available(), "the main mod alone has no video backend");
        SystemApps.provision(disk, name -> bin.resolve(name + ".wasm"), store);
        assertFalse(Files.exists(app), "no backend: no Video Player");

        VideoBackends.setForTesting(FAKE);
        SystemApps.provision(disk, name -> bin.resolve(name + ".wasm"), store);
        assertTrue(Files.isRegularFile(app.resolve("main.wasm")) && Files.isRegularFile(app.resolve("app.toml")), "installed with a backend");
        assertTrue(Files.readString(app.resolve("app.toml")).contains("open_with   = [\".mp4\""));
        assertEquals(Boolean.TRUE, store.decision("dev.ecm.video", "peripherals:speaker"));

        VideoBackends.setForTesting(null);
        SystemApps.provision(disk, name -> bin.resolve(name + ".wasm"), store);
        assertFalse(Files.exists(app), "the backend gone: the Video Player is removed");

        // Control: a folder that isn't the provisioner's copy (the player edited its manifest) stays.
        VideoBackends.setForTesting(FAKE);
        SystemApps.provision(disk, name -> bin.resolve(name + ".wasm"), store);
        Files.writeString(app.resolve("app.toml"), Files.readString(app.resolve("app.toml")) + "# mine\n");
        VideoBackends.setForTesting(null);
        SystemApps.provision(disk, name -> bin.resolve(name + ".wasm"), store);
        assertTrue(Files.exists(app.resolve("app.toml")), "an edited copy is left alone");
    }
}
