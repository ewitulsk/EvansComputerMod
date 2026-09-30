package com.example.evanscomputermod.testing.mixin;

//? if <=1.21.1 {
import net.minecraft.client.MouseHandler;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(MouseHandler.class)
public abstract class HiddenMouseMixin {
  @Shadow private boolean mouseGrabbed;

  @Inject(method = "grabMouse", at = @At("HEAD"), cancellable = true)
  private void virtualGrab(CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks")) {
      mouseGrabbed = true;
      ci.cancel();
    }
  }

  @Inject(method = "releaseMouse", at = @At("HEAD"), cancellable = true)
  private void virtualRelease(CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks")) {
      mouseGrabbed = false;
      ci.cancel();
    }
  }

  @Inject(method = "turnPlayer", at = @At("HEAD"), cancellable = true)
  private void scriptedLook(CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks")) ci.cancel();
  }
}
//?}
