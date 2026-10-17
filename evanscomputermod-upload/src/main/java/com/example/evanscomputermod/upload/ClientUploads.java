package com.example.evanscomputermod.upload;

import com.example.evanscomputermod.api.upload.UploadStatus;
import net.minecraft.client.Minecraft;
import org.lwjgl.util.tinyfd.TinyFileDialogs;
import org.slf4j.Logger;
import org.slf4j.LoggerFactory;

import java.io.IOException;
import java.nio.file.Files;
import java.nio.file.Path;

/**
 * The player's side of an upload, in their game. When a computer asks for a
 * file ({@link UploadPackets.Pick}), the system's own file picker opens
 * (LWJGL tinyfiledialogs: the Windows dialog, zenity/kdialog on Linux,
 * the macOS panel) on a thread of its own so the game keeps running. The
 * picked file is read and sent in {@link UploadPackets#CHUNK}-byte pieces,
 * {@link #CHUNKS_PER_TICK} per client tick. Chat lines tell the player what
 * happened.
 *
 * <p>For automated client checks only, the system property
 * {@code evanscomputermod.upload.pick} names a file to pick instead of
 * showing the dialog ({@code -} = the player cancels).
 */
public final class ClientUploads {
    private static final Logger LOGGER = LoggerFactory.getLogger(ClientUploads.class);
    /** ~4.8 MB/s at 20 ticks per second. */
    static final int CHUNKS_PER_TICK = 8;
    static final String TEST_PICK = "evanscomputermod.upload.pick";

    /** The running upload (client thread). */
    private static final class Active {
        final UploadPackets.Pick request;
        byte[] data;
        String name;
        int sent;
        boolean ended;

        Active(UploadPackets.Pick request) {
            this.request = request;
        }
    }

    private static Active active;

    private ClientUploads() {}

    static void onPick(UploadPackets.Pick m) {
        if (active != null) {
            ClientCompat.sendToServer(new UploadPackets.Cancel(m.ticket(), UploadStatus.CANCELLED));
            ClientCompat.clientMessage("Upload: finish the upload you started before starting another.");
            return;
        }
        Active a = new Active(m);
        active = a;
        ClientCompat.clientMessage("Upload: pick a file on your computer to send to the " + m.computerKind() + " (" + m.destination() + ").");
        Thread t = new Thread(() -> pick(a), "ECM-Upload-Picker");
        t.setDaemon(true);
        t.start();
    }

    /** The picker thread: ask for a file, read it, hand it to the client thread. */
    private static void pick(Active a) {
        String chosen;
        String test = System.getProperty(TEST_PICK);
        if (test != null) {
            chosen = test.equals("-") ? null : test;
        } else {
            try {
                chosen = TinyFileDialogs.tinyfd_openFileDialog("Upload a file to the " + a.request.computerKind(),
                        System.getProperty("user.home") + java.io.File.separator, null, null, false);
            } catch (Throwable e) {
                LOGGER.warn("[upload] the file picker failed", e);
                chosen = null;
            }
        }
        byte[] data = null;
        int status = UploadStatus.CANCELLED;
        String message = "Upload cancelled.";
        if (chosen != null) {
            Path file = Path.of(chosen);
            try {
                long size = Files.size(file);
                if (size > a.request.maxBytes()) {
                    status = UploadStatus.TOO_LARGE;
                    message = "Upload: " + file.getFileName() + " is " + human(size) + "; this server takes files up to "
                            + human(a.request.maxBytes()) + ".";
                } else {
                    data = Files.readAllBytes(file);
                }
            } catch (IOException | RuntimeException e) {
                status = UploadStatus.FAILED;
                message = "Upload: can't read " + chosen + " (" + e.getMessage() + ").";
            }
        }
        byte[] picked = data;
        int failStatus = status;
        String failMessage = message;
        String name = chosen == null ? null : Path.of(chosen).getFileName().toString();
        Minecraft.getInstance().execute(() -> picked(a, name, picked, failStatus, failMessage));
    }

    private static void picked(Active a, String name, byte[] data, int failStatus, String failMessage) {
        if (active != a) return; // the computer stopped waiting, or the player left
        if (data == null) {
            ClientCompat.sendToServer(new UploadPackets.Cancel(a.request.ticket(), failStatus));
            ClientCompat.clientMessage(failMessage);
            active = null;
            return;
        }
        a.data = data;
        a.name = name;
        ClientCompat.sendToServer(new UploadPackets.Begin(a.request.ticket(), name, data.length));
        ClientCompat.clientMessage("Upload: sending " + name + " (" + human(data.length) + ") to the " + a.request.computerKind() + "...");
    }

    /** Client tick: send the next pieces. */
    static void tick() {
        Active a = active;
        if (a == null || a.data == null || a.ended) return;
        for (int i = 0; i < CHUNKS_PER_TICK && a.sent < a.data.length; i++) {
            int n = Math.min(UploadPackets.CHUNK, a.data.length - a.sent);
            byte[] chunk = new byte[n];
            System.arraycopy(a.data, a.sent, chunk, 0, n);
            ClientCompat.sendToServer(new UploadPackets.Chunk(a.request.ticket(), chunk));
            a.sent += n;
        }
        if (a.sent >= a.data.length) {
            ClientCompat.sendToServer(new UploadPackets.End(a.request.ticket()));
            a.ended = true;
        }
    }

    static void onResult(UploadPackets.Result m) {
        Active a = active;
        if (a == null || a.request.ticket() != m.ticket()) return;
        active = null;
        if (m.status() == UploadStatus.DONE) {
            ClientCompat.clientMessage("Upload: " + (a.name == null ? "the file" : a.name) + " is on the " + a.request.computerKind()
                    + " at " + m.path() + ".");
        } else {
            String why = UploadStatus.describe(m.status());
            ClientCompat.clientMessage("Upload failed: " + (why == null ? "error " + m.status() : why) + ".");
        }
    }

    /** Left the server: drop whatever was running. */
    static void reset() {
        active = null;
    }

    static String human(long bytes) {
        if (bytes < 1024) return bytes + " B";
        if (bytes < 1024 * 1024) return String.format(java.util.Locale.ROOT, "%.1f KiB", bytes / 1024.0);
        return String.format(java.util.Locale.ROOT, "%.1f MiB", bytes / (1024.0 * 1024.0));
    }
}
