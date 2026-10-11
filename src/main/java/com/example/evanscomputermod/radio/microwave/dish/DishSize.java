package com.example.evanscomputermod.radio.microwave.dish;

/**
 * The three dishes. A dish of face {@code n}×{@code n} blocks occupies one
 * vertical square across its facing: parts are numbered {@code up * n + column},
 * columns run along the facing's clockwise side (the placer's right when
 * standing behind it). The controller part holds the block entity: the bottom
 * left for 1×1 and 2×2, the bottom centre for 3×3.
 */
public enum DishSize {
    SMALL("dish_small", 1, 0.6),
    MEDIUM("dish_medium", 2, 1.2),
    LARGE("dish_large", 3, 2.4);

    public final String id;
    public final int n;
    public final double diameterM;

    DishSize(String id, int n, double diameterM) {
        this.id = id;
        this.n = n;
        this.diameterM = diameterM;
    }

    public int parts() {
        return n * n;
    }

    public int controllerPart() {
        return n == 3 ? 1 : 0;
    }

    /** Offset of {@code part} from the controller along the clockwise side, blocks. */
    public int rightOf(int part) {
        return part % n - (n == 3 ? 1 : 0);
    }

    /** Offset of {@code part} above the controller, blocks. */
    public int upOf(int part) {
        return part / n;
    }

    /** Dish face centre relative to the controller block's centre: {right, up} in blocks. */
    public double centreRight() {
        return n == 2 ? 0.5 : 0;
    }

    public double centreUp() {
        return (n - 1) / 2.0;
    }
}
