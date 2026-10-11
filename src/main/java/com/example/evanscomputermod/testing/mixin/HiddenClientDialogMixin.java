package com.example.evanscomputermod.testing.mixin;

//? if <=1.21.1 {
import net.minecraft.client.Minecraft;
import org.spongepowered.asm.mixin.Mixin;
import org.spongepowered.asm.mixin.injection.*;
import org.spongepowered.asm.mixin.injection.callback.CallbackInfo;

@Mixin(Minecraft.class)
public abstract class HiddenClientDialogMixin {
  @Inject(
      method = "<init>",
      at =
          @At(
              value = "INVOKE",
              target =
                  "Lorg/lwjgl/util/tinyfd/TinyFileDialogs;tinyfd_messageBox(Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/CharSequence;Ljava/lang/CharSequence;Z)Z"))
  private void noDesktopDialog(CallbackInfo ci) {
    if (Boolean.getBoolean("ecm.clientChecks"))
      throw new IllegalStateException("Hidden client framebuffer dimensions do not match");
  }
}
//?}
