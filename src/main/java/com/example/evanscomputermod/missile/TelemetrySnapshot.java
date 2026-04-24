package com.example.evanscomputermod.missile;

/**
 * Immutable per-tick telemetry snapshot for an in-flight missile.
 * Stored in {@link MissileTelemetryStore} and read by the Python API.
 */
public record TelemetrySnapshot(
        double x,
        double y,
        double z,
        double vx,
        double vy,
        double vz,
        float fuelPct,
        String status,  // "thrust" | "coast" | "guiding" | "impact" | "lost"
        float tta       // estimated seconds to impact; -1 if unknown
) {

    /** Serialize to a JSON string for return to Python via the binary protocol. */
    public String toJson() {
        return String.format(
                java.util.Locale.ROOT,
                "{\"x\":%.2f,\"y\":%.2f,\"z\":%.2f," +
                "\"vx\":%.4f,\"vy\":%.4f,\"vz\":%.4f," +
                "\"fuel_pct\":%.3f,\"status\":\"%s\",\"tta\":%.1f}",
                x, y, z, vx, vy, vz, fuelPct, status, tta
        );
    }
}
