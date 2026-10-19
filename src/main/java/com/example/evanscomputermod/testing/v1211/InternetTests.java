package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.RouterScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Internet Gateway's link to the real network, namespace
 * {@code ecm_internet} (1.21.1): {@code router_internet}. Needs the
 * evanscomputermod-internet sidecar in the run (scripts/Test.ps1 -Internet);
 * without it the scenario stops at its first step and says so.
 */
@GameTestHolder(InternetTests.NS)
@PrefixGameTestTemplate(false)
public final class InternetTests {
    static final String NS = "ecm_internet";

    /** A client cabled to an Internet Gateway curls a web page on this machine; control: a LAN address is refused. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".router_internet")
    public static void router_internet(GameTestHelper h) {
        TestDriver.scenario(h, NS, RouterScenarios.ALL.get("router_internet"));
    }

    /**
     * {@code internet.gatewayRecipe}: off (the default), no recipe makes an Internet Gateway; on and
     * reloaded, the recipe crafts one; off and reloaded, it is gone again. The generated gateway can't
     * be broken and drops nothing; a placed (crafted) one breaks with a pickaxe and drops itself.
     */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".gateway_recipe")
    public static void gateway_recipe_follows_config(GameTestHelper h) {
        var server = h.getLevel().getServer();
        var level = h.getLevel();
        String[] failure = {null};
        int[] step = {0};
        java.util.concurrent.CompletableFuture<?>[] reload = {null};
        if (craftGateway(level).getCount() != 0) failure[0] = "a gateway recipe exists while internet.gatewayRecipe is off";

        var gateway = com.example.evanscomputermod.block.ModBlocks.INTERNET_GATEWAY.get();
        var natural = gateway.defaultBlockState();
        var crafted = natural.setValue(com.example.evanscomputermod.block.InternetGatewayBlock.CRAFTED, true);
        var player = h.makeMockPlayer(net.minecraft.world.level.GameType.SURVIVAL);
        var pick = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.DIAMOND_PICKAXE);
        player.setItemInHand(net.minecraft.world.InteractionHand.MAIN_HAND, pick);
        var pos = h.absolutePos(new net.minecraft.core.BlockPos(2, 2, 2));
        if (natural.getDestroyProgress(player, level, pos) != 0f) failure[0] = "the generated gateway can be broken";
        else if (!(crafted.getDestroyProgress(player, level, pos) > 0f)) failure[0] = "a crafted gateway can't be broken";
        else if (!net.minecraft.world.level.block.Block.getDrops(natural, level, pos, null, player, pick).isEmpty())
            failure[0] = "the generated gateway drops an item";
        else if (net.minecraft.world.level.block.Block.getDrops(crafted, level, pos, null, player, pick).stream()
                .noneMatch(st -> st.is(gateway.asItem()))) failure[0] = "a crafted gateway doesn't drop itself";

        TestDriver.drive(h, NS, "gateway_recipe_follows_config", () -> {
            if (failure[0] != null) return false;
            if (reload[0] != null && !reload[0].isDone()) return false;
            switch (step[0]) {
                case 0 -> {
                    com.example.evanscomputermod.EcmConfig.overrideInternetGatewayRecipe(true);
                    reload[0] = server.reloadResources(server.getPackRepository().getSelectedIds());
                    step[0] = 1;
                }
                case 1 -> {
                    var out = craftGateway(h.getLevel());
                    if (!out.is(gateway.asItem())) failure[0] = "enabled and reloaded, the recipe made " + out;
                    com.example.evanscomputermod.EcmConfig.overrideInternetGatewayRecipe(null);
                    reload[0] = server.reloadResources(server.getPackRepository().getSelectedIds());
                    step[0] = 2;
                }
                case 2 -> {
                    if (craftGateway(h.getLevel()).getCount() != 0) failure[0] = "control: off again, the recipe still exists";
                    return true;
                }
                default -> { }
            }
            return false;
        }, () -> failure[0]);
    }

    private static net.minecraft.world.item.ItemStack craftGateway(net.minecraft.server.level.ServerLevel level) {
        net.minecraft.world.item.ItemStack i = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.IRON_INGOT),
                c = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.COMPARATOR),
                n = new net.minecraft.world.item.ItemStack(com.example.evanscomputermod.block.ModBlocks.NETWORK_CABLE_ITEM.get()),
                e = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.ENDER_PEARL),
                r = new net.minecraft.world.item.ItemStack(net.minecraft.world.item.Items.REDSTONE_BLOCK);
        var input = net.minecraft.world.item.crafting.CraftingInput.of(3, 3, java.util.List.of(i, n, i, c, e, c, i, r, i));
        return level.getServer().getRecipeManager()
                .getRecipeFor(net.minecraft.world.item.crafting.RecipeType.CRAFTING, input, level)
                .map(h -> h.value().assemble(input, level.registryAccess()))
                .orElse(net.minecraft.world.item.ItemStack.EMPTY);
    }

    private InternetTests() {}
}
//?}
