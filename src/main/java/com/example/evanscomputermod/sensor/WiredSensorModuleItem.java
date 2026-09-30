package com.example.evanscomputermod.sensor;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.module.IComputerModule;
import com.example.evanscomputermod.api.module.IComputerModuleItem;
import com.example.evanscomputermod.api.module.IModuleHost;
import com.example.evanscomputermod.api.module.ModuleSlotVisual;
import com.example.evanscomputermod.item.TooltipItem;
import com.example.evanscomputermod.module.ModuleState;
import net.minecraft.ChatFormatting;
import net.minecraft.nbt.CompoundTag;
import net.minecraft.network.chat.Component;
import net.minecraft.world.item.ItemStack;

import java.util.function.Consumer;

/**
 * Wired Sensor Module item: install it in a computer's module bay, then run
 * Sensor Wire from the connector on its cartridge to up to eight sensors.
 */
public class WiredSensorModuleItem extends TooltipItem implements IComputerModuleItem {

    public WiredSensorModuleItem(Properties properties) {
        super(properties);
    }

    @Override
    public IComputerModule createModule(IModuleHost host, ItemStack stack, CompoundTag savedState) {
        return new WiredSensorModule(host, savedState);
    }

    @Override
    public ModuleSlotVisual getSlotVisual(ItemStack stack) {
        return ModuleSlotVisual.WIRED_SENSOR;
    }

    @Override
    protected void addTooltip(ItemStack stack, Consumer<Component> lines) {
        lines.accept(Component.translatable("item.evanscomputermod.wired_sensor_module.tooltip")
                .withStyle(ChatFormatting.GRAY));
        CompoundTag state = ModuleState.read(stack);
        int named = state.getCompound("names").size();
        if(named > 0) {
            lines.accept(Component.translatable("item.evanscomputermod.wired_sensor_module.known", named)
                    .withStyle(ChatFormatting.DARK_GRAY));
        }
    }
}
//?}
