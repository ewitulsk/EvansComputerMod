plugins {
    id("dev.kikugie.stonecutter")
}
stonecutter active "1.21.1"

// Aggregate `build` across every configured stonecutter version. This lets
// `./gradlew chiseledBuild` produce every MC-version jar in one invocation.
// Stonecutter 0.9's own chiseled-task DSL isn't used here; a plain aggregator
// task that depends on each subproject's `build` achieves the same result.
tasks.register("chiseledBuild") {
    group = "project"
    description = "Builds jars for all configured MC versions."
    dependsOn(subprojects.map { "${it.path}:build" })
}
