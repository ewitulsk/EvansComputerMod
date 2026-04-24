package com.example.evanscomputermod.computer;

import com.example.evanscomputermod.api.ComputerContext;
import com.example.evanscomputermod.api.ComputerFunction;
import com.example.evanscomputermod.api.ComputerModule;
import com.example.evanscomputermod.block.MissileLauncherBlockEntity;
import com.example.evanscomputermod.missile.MissileTelemetryStore;
import com.example.evanscomputermod.missile.MissileTier;
import com.example.evanscomputermod.missile.TelemetrySnapshot;
import net.minecraft.core.BlockPos;
import net.minecraft.world.level.Level;

import org.jetbrains.annotations.Nullable;
import java.util.UUID;
import java.util.concurrent.ConcurrentHashMap;

/**
 * Python API module: {@code import missile}
 *
 * <p>All functions require a launcher to be assigned first via
 * {@code missile.set_launcher(x, y, z)}. State is tracked per-computer-UUID
 * so multiple terminals can each control their own launcher independently.
 *
 * <pre>
 * import missile, json
 *
 * missile.set_launcher(10, 64, 20)
 * missile.set_bearing(270.0)
 * missile.set_elevation(55.0)
 * missile.set_target(500, 64, -800)
 * missile.arm("lrbm")
 * launch_id = missile.launch()
 *
 * while True:
 *     t = json.loads(missile.get_telemetry(launch_id))
 *     print(t['status'], t['x'], t['y'], t['z'])
 *     if t['status'] in ('impact', 'lost'):
 *         break
 * </pre>
 */
@ComputerModule(value = "missile", description = "Missile launcher control and telemetry")
public class MissileComputerModule {

    // Per-computer-UUID state maps (module is a singleton)
    private final ConcurrentHashMap<UUID, BlockPos> launcherPositions = new ConcurrentHashMap<>();
    private final ConcurrentHashMap<UUID, BlockPos> targetPositions   = new ConcurrentHashMap<>();
    private final ConcurrentHashMap<UUID, Float>    yieldOverrides    = new ConcurrentHashMap<>();

    /** Default scan radius (blocks) for launcher discovery. */
    private static final int SCAN_RADIUS = 32;

    // ── Launcher discovery ───────────────────────────────────────────────

    /**
     * Scan for missile launchers within {@value #SCAN_RADIUS} blocks of the
     * terminal.  Returns a string of newline-separated "x y z" coordinates,
     * or an empty string if none are found.
     */
    @ComputerFunction(description = "Find nearby missile launchers; returns newline-separated 'x y z' coords", mainThread = true)
    public String find_launchers(ComputerContext ctx) {
        Level level = ctx.getLevel();
        BlockPos origin = ctx.getPosition();
        if (level == null || origin == null) return "";

        StringBuilder sb = new StringBuilder();
        for (BlockPos pos : BlockPos.betweenClosed(
                origin.offset(-SCAN_RADIUS, -SCAN_RADIUS, -SCAN_RADIUS),
                origin.offset(SCAN_RADIUS, SCAN_RADIUS, SCAN_RADIUS))) {
            if (level.getBlockEntity(pos) instanceof MissileLauncherBlockEntity) {
                if (sb.length() > 0) sb.append('\n');
                sb.append(pos.getX()).append(' ').append(pos.getY()).append(' ').append(pos.getZ());
            }
        }
        return sb.toString();
    }

    /**
     * Auto-connect to the nearest missile launcher within
     * {@value #SCAN_RADIUS} blocks. Returns true if one was found.
     */
    @ComputerFunction(description = "Connect to the nearest missile launcher", mainThread = true)
    public boolean connect(ComputerContext ctx) {
        Level level = ctx.getLevel();
        BlockPos origin = ctx.getPosition();
        if (level == null || origin == null) return false;

        BlockPos best = null;
        double bestDist = Double.MAX_VALUE;
        for (BlockPos pos : BlockPos.betweenClosed(
                origin.offset(-SCAN_RADIUS, -SCAN_RADIUS, -SCAN_RADIUS),
                origin.offset(SCAN_RADIUS, SCAN_RADIUS, SCAN_RADIUS))) {
            if (level.getBlockEntity(pos) instanceof MissileLauncherBlockEntity) {
                double d = origin.distSqr(pos);
                if (d < bestDist) {
                    bestDist = d;
                    best = pos.immutable();
                }
            }
        }
        if (best == null) return false;
        launcherPositions.put(ctx.getComputerId(), best);
        return true;
    }

    // ── Launcher selection ────────────────────────────────────────────────

    /**
     * Assign a launcher block at the given world coordinates to this terminal.
     * Returns true if a launcher block entity exists there, false otherwise.
     */
    @ComputerFunction(description = "Assign a launcher block by world coordinates", mainThread = true)
    public boolean set_launcher(ComputerContext ctx, int x, int y, int z) {
        Level level = ctx.getLevel();
        if (level == null) return false;

        BlockPos pos = new BlockPos(x, y, z);
        if (!(level.getBlockEntity(pos) instanceof MissileLauncherBlockEntity)) {
            return false;
        }
        launcherPositions.put(ctx.getComputerId(), pos);
        return true;
    }

    // ── Orientation ───────────────────────────────────────────────────────

    /** Set barrel bearing (0–360, 0 = North, 90 = East). */
    @ComputerFunction(description = "Set launch bearing in degrees", mainThread = true)
    public boolean set_bearing(ComputerContext ctx, double bearing) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;
        launcher.setBearing((float) bearing);
        return true;
    }

    /** Set barrel elevation (5–85 degrees above horizontal). */
    @ComputerFunction(description = "Set launch elevation in degrees", mainThread = true)
    public boolean set_elevation(ComputerContext ctx, double elevation) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;
        launcher.setElevation((float) elevation);
        return true;
    }

    // ── Targeting ─────────────────────────────────────────────────────────

    /**
     * Set GPS target world coordinates. Automatically calculates and applies
     * the bearing and elevation from the launcher to the target.
     */
    @ComputerFunction(description = "Set GPS target and auto-aim the launcher", mainThread = true)
    public boolean set_target(ComputerContext ctx, int x, int y, int z) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;
        BlockPos target = new BlockPos(x, y, z);
        launcher.setTarget(target);
        targetPositions.put(ctx.getComputerId(), target);

        // Auto-aim: compute bearing and elevation from launcher to target
        BlockPos launcherPos = launcher.getMountPos();
        double dx = x - launcherPos.getX();
        double dy = y - launcherPos.getY();
        double dz = z - launcherPos.getZ();
        double horizontalDist = Math.sqrt(dx * dx + dz * dz);

        // Bearing: atan2(dx, -dz) gives 0=North(−Z), 90=East(+X)
        float bearing = (float) Math.toDegrees(Math.atan2(dx, -dz));
        if (bearing < 0) bearing += 360f;
        launcher.setBearing(bearing);

        // Elevation: aim upward based on distance — 45° is a good default
        // for ballistic range; steeper for close targets, flatter for far ones.
        float elevation = (float) Math.toDegrees(Math.atan2(dy + horizontalDist * 0.3, horizontalDist));
        elevation = Math.max(5f, Math.min(85f, elevation));
        launcher.setElevation(elevation);

        return true;
    }

    /** Clear the GPS target (unguided flight). */
    @ComputerFunction(description = "Clear GPS target for unguided flight", mainThread = true)
    public boolean clear_target(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;
        launcher.setTarget(null);
        targetPositions.remove(ctx.getComputerId());
        return true;
    }

    // ── Yield ─────────────────────────────────────────────────────────────

    /**
     * Override warhead yield (0.0 = minimum, 1.0 = maximum).
     * Defaults to the tier's built-in yield if not set.
     */
    @ComputerFunction(description = "Set warhead yield scale (0.0–1.0)", mainThread = true)
    public boolean set_yield(ComputerContext ctx, double scale) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;
        float f = (float) scale;
        launcher.setYieldOverride(f);
        yieldOverrides.put(ctx.getComputerId(), f);
        return true;
    }

    // ── Arming ────────────────────────────────────────────────────────────

    /**
     * Load a missile tier into the launcher.
     * Accepts: "mrbm", "lrbm", "icbm" (case-insensitive).
     * Returns true if the tier is valid and the launcher accepted it.
     */
    @ComputerFunction(description = "Load a missile tier into the launcher", mainThread = true)
    public boolean arm(ComputerContext ctx, String tierName) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null) return false;

        MissileTier tier = MissileTier.fromId(tierName.toLowerCase());
        launcher.loadMissile(tier);
        return true;
    }

    /** Returns true if a missile is currently loaded in the assigned launcher. */
    @ComputerFunction(description = "Check if the launcher has a missile loaded", mainThread = true)
    public boolean is_armed(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        return launcher != null && launcher.isLoaded();
    }

    // ── Launch ────────────────────────────────────────────────────────────

    /**
     * Fire the loaded missile.
     *
     * @return A UUID string used to poll telemetry, or an empty string on failure.
     */
    @ComputerFunction(description = "Launch the loaded missile; returns telemetry ID string", mainThread = true)
    public String launch(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null || !launcher.isLoaded()) return "";

        UUID launchId = UUID.randomUUID();
        var missile = launcher.triggerLaunch(launchId);
        if (missile == null) return "";

        return launchId.toString();
    }

    // ── Telemetry ─────────────────────────────────────────────────────────

    /**
     * Get the latest telemetry snapshot for a missile, as a JSON string.
     * Returns an empty string if the launch ID is unknown.
     *
     * <p>Poll in a loop; typically arrives within one tick (~50 ms).
     * The {@code "status"} field changes to {@code "impact"} or {@code "lost"}
     * on terminal events.
     */
    @ComputerFunction(description = "Get telemetry JSON for a launch ID; poll until status = impact or lost")
    public String get_telemetry(ComputerContext ctx, String launchIdStr) {
        UUID id;
        try {
            id = UUID.fromString(launchIdStr);
        } catch (IllegalArgumentException e) {
            return "";
        }

        TelemetrySnapshot snapshot = MissileTelemetryStore.get(id);
        return snapshot != null ? snapshot.toJson() : "";
    }

    /**
     * Clear a completed telemetry entry from the store to free memory.
     * Call once you've processed the final "impact" or "lost" snapshot.
     */
    @ComputerFunction(description = "Remove a completed flight from telemetry store")
    public void clear_telemetry(ComputerContext ctx, String launchIdStr) {
        try {
            MissileTelemetryStore.remove(UUID.fromString(launchIdStr));
        } catch (IllegalArgumentException ignored) {}
    }

    // ── Status query ──────────────────────────────────────────────────────

    /** Returns current bearing of the assigned launcher, or -1 if none assigned. */
    @ComputerFunction(description = "Get current bearing of the assigned launcher", mainThread = true)
    public double get_bearing(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        return launcher != null ? launcher.getBearing() : -1.0;
    }

    /** Returns current elevation of the assigned launcher, or -1 if none assigned. */
    @ComputerFunction(description = "Get current elevation of the assigned launcher", mainThread = true)
    public double get_elevation(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        return launcher != null ? launcher.getElevation() : -1.0;
    }

    /** Returns the loaded tier name ("mrbm"/"lrbm"/"icbm"), or empty string if empty. */
    @ComputerFunction(description = "Get the loaded missile tier name", mainThread = true)
    public String get_loaded_tier(ComputerContext ctx) {
        MissileLauncherBlockEntity launcher = getLauncher(ctx);
        if (launcher == null || !launcher.isLoaded()) return "";
        MissileTier tier = launcher.getLoadedTier();
        return tier != null ? tier.id : "";
    }

    // ── Internal helper ───────────────────────────────────────────────────

    @Nullable
    private MissileLauncherBlockEntity getLauncher(ComputerContext ctx) {
        BlockPos pos = launcherPositions.get(ctx.getComputerId());
        if (pos == null || ctx.getLevel() == null) return null;
        if (ctx.getLevel().getBlockEntity(pos) instanceof MissileLauncherBlockEntity launcher) {
            return launcher;
        }
        // Block was replaced — remove stale entry
        launcherPositions.remove(ctx.getComputerId());
        return null;
    }
}
