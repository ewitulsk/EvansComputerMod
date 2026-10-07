package com.example.evanscomputermod.sensor.mixin;

//? if <=1.21.1 {
import com.example.evanscomputermod.controller.client.ControllerClient;
import net.minecraft.client.KeyboardHandler;
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.At;
import org.spongepowered.asm.mixin.injection.Inject;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/**
 * Escape while a Wireless Controller is connected in the world only
 * disconnects it. Vanilla hard-codes Escape to the pause menu (it is not a
 * key mapping), so it is caught before the keyboard handler sees it.
 */
@Mixin(KeyboardHandler.class)
public abstract class ControllerEscapeMixin {
    @Inject(method = "keyPress", at = @At("HEAD"), cancellable = true)
    private void evanscomputermod$escapeDisconnectsController(long window, int key, int scancode, int action,
                                                              int modifiers, CallbackInfo ci) {
        Minecraft mc = Minecraft.getInstance();
        if (key == 256 && action == 1 && window == mc.getWindow().getWindow() && mc.screen == null
                && ControllerClient.disconnectOnEscape()) {
            ci.cancel();
        }
    }
}
//?}
