package com.example.evanscomputermod.controller;

import com.example.evanscomputermod.computer.peripheral.PeripheralEventBus;
import com.example.evanscomputermod.computer.peripheral.PeripheralHub;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.world.level.Level;
import org.junit.jupiter.api.Test;

import java.util.EnumSet;
import java.util.List;
import java.util.UUID;

import static org.junit.jupiter.api.Assertions.*;

/** Connecting, numbering and timing out wireless controllers. */
class WirelessControllerHubTest {

    private final PeripheralHub hub = new PeripheralHub(new PeripheralHub.Owner() {
        @Override public Level level() { return null; }
        @Override public BlockPos pos() { return BlockPos.ZERO; }
        @Override public Direction facing() { return Direction.NORTH; }
        @Override public UUID computerId() { return UUID.randomUUID(); }
        @Override public PeripheralEventBus events() { return null; }
    });
    private final WirelessControllerHub controllers = new WirelessControllerHub(hub);
    private final UUID player = UUID.randomUUID();

    @Test
    void controllersGetPlayerNumbersInConnectOrderUpToFour() {
        UUID[] ids = new UUID[5];
        for (int i = 0; i < ids.length; i++) ids[i] = UUID.randomUUID();
        for (int i = 0; i < 4; i++) {
            assertEquals(i + 1, controllers.update(ids[i], player, ControllerState.NEUTRAL, 0));
        }
        assertEquals(0, controllers.update(ids[4], player, ControllerState.NEUTRAL, 0), "a fifth doesn't fit");
        assertEquals(List.of("controller_1", "controller_2", "controller_3", "controller_4"), hub.names());

        // Player 2 leaves; the next controller takes its number.
        assertTrue(controllers.disconnect(ids[1]));
        assertEquals(2, controllers.update(ids[4], player, ControllerState.NEUTRAL, 0));
    }

    @Test
    void silentControllersTimeOutAndReleaseTheirButtons() {
        UUID id = UUID.randomUUID();
        ControllerState aDown = ControllerState.of(EnumSet.of(ControllerInput.A));
        controllers.update(id, player, aDown, 1_000);
        ControllerPeripheral pad = (ControllerPeripheral) hub.get("controller_1");
        assertTrue(pad.state().isDown(ControllerInput.Button.A));

        controllers.tick(1_000 + WirelessControllerHub.TIMEOUT_MS);
        assertEquals(1, controllers.connectedCount(), "not yet");
        controllers.tick(1_001 + WirelessControllerHub.TIMEOUT_MS);
        assertEquals(0, controllers.connectedCount());
        assertNull(hub.get("controller_1"));
        assertFalse(pad.state().isDown(ControllerInput.Button.A), "a dropped controller releases everything");
    }

    @Test
    void keysBecomeButtonsAndDigitalAxes() {
        ControllerState s = ControllerState.of(EnumSet.of(
                ControllerInput.LS_UP, ControllerInput.LS_LEFT, ControllerInput.RS_LEFT, ControllerInput.RS_RIGHT,
                ControllerInput.RT, ControllerInput.START));
        assertEquals(127, s.ly());
        assertEquals(-127, s.lx());
        assertEquals(0, s.rx(), "opposite directions cancel");
        assertEquals(255, s.rt());
        assertEquals(0, s.lt());
        assertTrue(s.isDown(ControllerInput.Button.START));
        assertEquals(1.0, s.axis("ly"));
    }

    @Test
    void networkValuesAreClamped() {
        ControllerState s = new ControllerState(0xFFFF, -500, 500, 0, 0, 999, -1);
        assertEquals(ControllerState.BUTTON_MASK, s.buttons());
        assertEquals(-127, s.lx());
        assertEquals(127, s.ly());
        assertEquals(255, s.lt());
        assertEquals(0, s.rt());
    }

    @Test
    void keyNamesAreValidated() {
        assertTrue(ControllerData.isValidKeyName("key.keyboard.j"));
        assertTrue(ControllerData.isValidKeyName("key.keyboard.keypad.8"));
        assertTrue(ControllerData.isValidKeyName(""));
        assertFalse(ControllerData.isValidKeyName("key.keyboard.J"));
        assertFalse(ControllerData.isValidKeyName("minecraft:stone"));
        assertFalse(ControllerData.isValidKeyName("key.keyboard." + "a".repeat(80)));
    }

    @Test
    void everyInputHasADistinctIdAndDefaultKey() {
        var ids = new java.util.HashSet<String>();
        var keys = new java.util.HashSet<String>();
        for (ControllerInput in : ControllerInput.values()) {
            assertTrue(ids.add(in.id()), in.id());
            assertTrue(ControllerData.isValidKeyName(in.defaultKey()), in.defaultKey());
            if (!in.defaultKey().isEmpty()) assertTrue(keys.add(in.defaultKey()), "duplicate default " + in.defaultKey());
        }
    }
}
