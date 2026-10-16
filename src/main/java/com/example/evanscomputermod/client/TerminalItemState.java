package com.example.evanscomputermod.client;

//? if >=26.1 {
import com.example.evanscomputermod.EvansComputerMod;
import com.example.evanscomputermod.block.TerminalBlock;
import com.mojang.serialization.Codec;
import com.mojang.serialization.codecs.RecordCodecBuilder;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.item.properties.select.SelectItemModelProperty;
import net.minecraft.resources.Identifier;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.BlockItem;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;
import net.minecraft.world.level.block.state.properties.Property;
import org.jspecify.annotations.Nullable;

/**
 * Item model select property {@code evanscomputermod:terminal_state} (26.1):
 * the value of one block-state property ({@code "block_state_property": "panel"},
 * {@code "left_bay_1"}, ...) in the state a Terminal item will place as
 * ({@link TerminalBlock#itemState}). {@code items/terminal_block.json} selects
 * the block's multipart parts with it (scripts/gen-module-models.py), so
 * the item shows its installed panel, bay cards and modules like the placed
 * block. On 1.21.1, {@link TerminalItemModel} does the same.
 */
public record TerminalItemState(String property) implements SelectItemModelProperty<String> {

    public static final Identifier ID = Identifier.fromNamespaceAndPath(EvansComputerMod.MODID, "terminal_state");
    public static final SelectItemModelProperty.Type<TerminalItemState, String> TYPE = SelectItemModelProperty.Type.create(
            RecordCodecBuilder.mapCodec(i -> i.group(
                    Codec.STRING.fieldOf("block_state_property").forGetter(TerminalItemState::property)
            ).apply(i, TerminalItemState::new)),
            Codec.STRING);

    @Override
    public @Nullable String get(ItemStack stack, @Nullable ClientLevel level, @Nullable LivingEntity owner,
                                int seed, ItemDisplayContext displayContext) {
        if (!(stack.getItem() instanceof BlockItem item)) return null;
        BlockState state = TerminalBlock.itemState(item.getBlock().defaultBlockState(), stack);
        Property<?> p = state.getBlock().getStateDefinition().getProperty(property);
        return p == null ? null : valueName(state, p);
    }

    private static <T extends Comparable<T>> String valueName(BlockState state, Property<T> property) {
        return property.getName(state.getValue(property));
    }

    @Override
    public SelectItemModelProperty.Type<TerminalItemState, String> type() {
        return TYPE;
    }

    @Override
    public Codec<String> valueCodec() {
        return Codec.STRING;
    }
}
//?}
