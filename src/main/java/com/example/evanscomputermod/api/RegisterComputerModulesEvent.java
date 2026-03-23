package com.example.evanscomputermod.api;

import net.neoforged.bus.api.Event;

/**
 * Fired during mod setup to allow third-party mods to register their
 * {@link ComputerModule} classes with the {@link ComputerModuleRegistry}.
 *
 * <pre>
 * {@literal @}SubscribeEvent
 * public void onRegisterModules(RegisterComputerModulesEvent event) {
 *     ComputerModuleRegistry.register(new GolemAPI());
 * }
 * </pre>
 */
public class RegisterComputerModulesEvent extends Event {
}
