package com.example.evanscomputermod.client;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.ModBlocks;
import com.example.evanscomputermod.block.TerminalBlock;
import com.mojang.blaze3d.vertex.PoseStack;
import net.minecraft.client.Minecraft;
import net.minecraft.client.multiplayer.ClientLevel;
import net.minecraft.client.renderer.RenderType;
import net.minecraft.client.renderer.block.model.BakedQuad;
import net.minecraft.client.renderer.block.model.ItemOverrides;
import net.minecraft.client.resources.model.BakedModel;
import net.minecraft.client.resources.model.ModelResourceLocation;
import net.minecraft.core.Direction;
import net.minecraft.core.registries.BuiltInRegistries;
import net.minecraft.util.RandomSource;
import net.minecraft.world.entity.LivingEntity;
import net.minecraft.world.item.ItemDisplayContext;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.state.BlockState;
import net.neoforged.neoforge.client.event.ModelEvent;
import net.neoforged.neoforge.client.model.BakedModelWrapper;
import net.neoforged.neoforge.client.model.data.ModelData;
import org.jetbrains.annotations.Nullable;

import java.util.List;
import java.util.Map;
import java.util.concurrent.ConcurrentHashMap;

/**
 * The Terminal item's model (1.21.1): it draws the block's own multipart model
 * in the state the item will place as ({@link TerminalBlock#itemState}), so the
 * item shows its installed panel, its bay cards and its modules exactly like
 * the placed block. Transforms (GUI, hand, ground) come from the item model
 * JSON. On 26.1 the item model definition selects the same parts through
 * {@link TerminalItemState}.
 */
public final class TerminalItemModel extends BakedModelWrapper<BakedModel> {

    private final Map<BlockState, BakedModel> byState = new ConcurrentHashMap<>();
    private final ItemOverrides overrides = new ItemOverrides() {
        @Override
        public BakedModel resolve(BakedModel model, ItemStack stack, @Nullable ClientLevel level,
                                  @Nullable LivingEntity entity, int seed) {
            BlockState state = TerminalBlock.itemState(ModBlocks.TERMINAL_BLOCK.get().defaultBlockState(), stack);
            return byState.computeIfAbsent(state, s -> new Fixed(originalModel,
                    Minecraft.getInstance().getBlockRenderer().getBlockModel(s), s));
        }
    };

    private TerminalItemModel(BakedModel itemModel) {
        super(itemModel);
    }

    @Override
    public ItemOverrides getOverrides() {
        return overrides;
    }

    /** Swaps the baked Terminal item model for this one. */
    public static void onModifyBakingResult(ModelEvent.ModifyBakingResult event) {
        ModelResourceLocation key = ModelResourceLocation.inventory(
                BuiltInRegistries.ITEM.getKey(ModBlocks.TERMINAL_BLOCK.get().asItem()));
        BakedModel model = event.getModels().get(key);
        if (model != null) event.getModels().put(key, new TerminalItemModel(model));
    }

    /** The item model's transforms with the block model's quads for one state. */
    private static final class Fixed extends BakedModelWrapper<BakedModel> {
        private final BakedModel block;
        private final BlockState state;

        Fixed(BakedModel itemModel, BakedModel block, BlockState state) {
            super(itemModel);
            this.block = block;
            this.state = state;
        }

        @Override
        public List<BakedQuad> getQuads(@Nullable BlockState ignored, @Nullable Direction side, RandomSource rand) {
            return block.getQuads(state, side, rand);
        }

        @Override
        public List<BakedQuad> getQuads(@Nullable BlockState ignored, @Nullable Direction side, RandomSource rand,
                                        ModelData data, @Nullable RenderType renderType) {
            // The item's render type is an item (entity) one, which the block's parts don't
            // list; the terminal's parts are all solid, so draw every quad in it.
            return block.getQuads(state, side, rand, ModelData.EMPTY, null);
        }

        @Override
        public ItemOverrides getOverrides() {
            return ItemOverrides.EMPTY;
        }

        // The item renderer draws the models these return; the wrapper's defaults would
        // hand back the item model. Apply the item model's transform but draw this one.
        @Override
        public BakedModel applyTransform(ItemDisplayContext context, PoseStack poseStack, boolean leftHand) {
            originalModel.applyTransform(context, poseStack, leftHand);
            return this;
        }

        @Override
        public List<BakedModel> getRenderPasses(ItemStack stack, boolean fabulous) {
            return List.of(this);
        }
    }
}
//?}
