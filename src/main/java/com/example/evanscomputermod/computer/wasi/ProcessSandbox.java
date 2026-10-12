package com.example.evanscomputermod.computer.wasi;

/**
 * The permission check behind the child host functions (docs/gui/APPS.md,
 * "Where permissions are enforced"). {@link ProcessManager#setSandbox}
 * installs one per computer (the app manager); host functions ask it with the
 * calling child's pid. Programs that were not launched as apps are
 * unconfined: every check passes for them, exactly as before.
 *
 * <p>{@link #check} may block the calling child thread while the player is
 * asked ({@code PermissionPrompter}); it is only called on child threads.
 * {@link #peek} never blocks (for checks on other threads, e.g. filtering
 * peripheral events as they are posted).
 */
public interface ProcessSandbox {

    /** True if {@code pid} runs as (or was spawned by) an app. */
    boolean isConfined(int pid);

    /** May {@code pid} use {@code permission}? Unconfined pids: always true. May prompt and block. */
    boolean check(int pid, String permission);

    /** Like {@link #check} but never prompts: true only if already allowed. */
    boolean peek(int pid, String permission);

    /**
     * May {@code pid} read (or write) the virtual path {@code vpath}
     * ({@code "/var/apps/x/notes.txt"}, relative to the computer's disk root)?
     */
    boolean checkPath(int pid, String vpath, boolean write);

    /** The permission guarding a peripheral type (UI_PROTOCOL.md §10). */
    static String peripheralPermission(String type) {
        if (type == null) return "peripherals:unknown";
        return switch (type) {
            case "sdr", "amplifier", "antenna", "dish", "microwave_radio", "wifi" -> "radio";
            case "lte_modem", "mmwave_modem", "framed_radio", "sim" -> "cellular";
            case "xbox_controller", "controller_receiver" -> "controller";
            default -> "peripherals:" + type;
        };
    }

    /** The permission guarding a {@code /dev/<name>} device, or null if none is needed. */
    static String devicePermission(String devName) {
        if (devName.startsWith("sdr") || devName.startsWith("frames")) return "radio";
        if (devName.startsWith("audio")) return "peripherals:speaker";
        return null;
    }

    /** Normalise a child path ({@code "apps/x/./a"}, {@code "/apps//x/"}) to {@code "/apps/x/a"}. */
    static String normalize(String path) {
        if (path == null) return "/";
        StringBuilder sb = new StringBuilder();
        for (String part : path.replace('\\', '/').split("/")) {
            if (part.isEmpty() || part.equals(".")) continue;
            if (part.equals("..")) {
                int cut = sb.lastIndexOf("/");
                sb.setLength(Math.max(cut, 0));
                continue;
            }
            sb.append('/').append(part);
        }
        return sb.length() == 0 ? "/" : sb.toString();
    }
}
