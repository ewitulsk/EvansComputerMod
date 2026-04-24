package com.example.evanscomputermod.item;

import com.example.evanscomputermod.missile.MissileTier;
import net.minecraft.world.item.Item;

/** Item representing one ready-to-load missile of a specific tier. */
public class MissileItem extends Item {

    private final MissileTier tier;

    public MissileItem(MissileTier tier, Properties properties) {
        super(properties);
        this.tier = tier;
    }

    public MissileTier getTier() {
        return tier;
    }
}
