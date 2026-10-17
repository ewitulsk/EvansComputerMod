package com.example.evanscomputermod.api.upload;

import com.example.evanscomputermod.EvansComputerMod;

/**
 * The installed {@link UploadBackend}, if any. The {@code evanscomputermod-upload}
 * sidecar installs one from its mod constructor; without it computers can't
 * take uploads.
 */
public final class UploadBackends {
    private static volatile UploadBackend installed;

    private UploadBackends() {}

    /** Called by the sidecar. A backend for another API version is refused. */
    public static synchronized void install(UploadBackend backend) {
        if (backend.apiVersion() != UploadBackend.CURRENT_API_VERSION) {
            EvansComputerMod.LOGGER.warn("Upload backend '{}' rejected: apiVersion {} != {}", backend.name(), backend.apiVersion(),
                    UploadBackend.CURRENT_API_VERSION);
            return;
        }
        installed = backend;
        EvansComputerMod.LOGGER.info("Upload backend: {}", backend.name());
    }

    /** The backend, or null when no upload support is installed. */
    public static UploadBackend get() {
        return installed;
    }

    public static boolean available() {
        return installed != null;
    }

    /** Tests: use this backend (null = none). */
    public static synchronized void setForTesting(UploadBackend backend) {
        installed = backend;
    }
}
