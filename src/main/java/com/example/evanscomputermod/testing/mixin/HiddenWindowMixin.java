package com.example.evanscomputermod.testing.mixin;

//? if <=1.21.1 {
import com.mojang.blaze3d.platform.Window;
import org.lwjgl.glfw.GLFW;
import org.spongepowered.asm.mixin.*;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

/** Opt-in isolated visual test process: never creates a visible or focused window. */
@Mixin(Window.class)
public abstract class HiddenWindowMixin {
  @Shadow private boolean fullscreen;

  @Inject(
      method = "<init>",
      at =
          @At(
              value = "INVOKE",
              target =
                  "Lnet/neoforged/fml/loading/ImmediateWindowHandler;setupMinecraftWindow(Ljava/util/function/IntSupplier;Ljava/util/function/IntSupplier;Ljava/util/function/Supplier;Ljava/util/function/LongSupplier;)J"))
  private void hidden(CallbackInfo ci) {
    if (!Boolean.getBoolean("ecm.clientChecks")) return;
    fullscreen = false;
    GLFW.glfwWindowHint(GLFW.GLFW_VISIBLE, GLFW.GLFW_FALSE);
    GLFW.glfwWindowHint(GLFW.GLFW_FOCUSED, GLFW.GLFW_FALSE);
    GLFW.glfwWindowHint(GLFW.GLFW_FOCUS_ON_SHOW, GLFW.GLFW_FALSE);
  }

  @Inject(method = "setMode", at = @At("HEAD"), cancellable = true)
  private void noMonitorChanges(CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks")) {
      fullscreen = false;
      ci.cancel();
    }
  }

  @Inject(method = "bootCrash", at = @At("HEAD"))
  private static void failWithoutDialog(int error, long description, CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks"))
      throw new IllegalStateException("Hidden client GLFW error " + error);
  }
}
//?}
