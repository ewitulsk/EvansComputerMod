package com.example.evanscomputermod.client;

import com.example.evanscomputermod.block.ScreenBlockEntity;
import com.example.evanscomputermod.block.TerminalBlockEntity;
import com.example.evanscomputermod.computer.TerminalDisplay;
import net.minecraft.core.BlockPos;
import com.example.evanscomputermod.network.LoadProgramResponsePacket;
import com.example.evanscomputermod.network.OpenVisualEditorPacket;
import com.example.evanscomputermod.network.ProgramListResponsePacket;
import com.example.evanscomputermod.network.DeltaApplier;
import com.example.evanscomputermod.network.TerminalDeltaPacket;
import com.example.evanscomputermod.network.TerminalOutputPacket;
import com.example.evanscomputermod.network.TerminalReadyPacket;
import net.minecraft.client.Minecraft;
import net.minecraft.world.level.Level;
import net.neoforged.neoforge.network.handling.IPayloadContext;
import net.neoforged.neoforge.client.network.ClientPacketDistributor;

import java.util.Arrays;
import java.util.List;

/**
 * Client-side packet handlers. This class is only loaded on the client,
 * keeping client-only imports (Minecraft, ClientLevel, etc.) off the server classpath.
 */
public final class ClientPacketHandler {

    public static void handleTerminalOutput(TerminalOutputPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;
            if (level == null) return;

            if (resolveBlockEntity(level, packet.pos()) instanceof TerminalBlockEntity te) {
                te.getDisplay().setFromBytes(packet.framebufferData());
            }
        });
    }

    /**
     * BE resolver that falls back to sable plot chunks on 1.21.1. The
     * terminal or anchor BlockPos carried in a packet may live inside a
     * physics plot (far from world origin), so the regular client-level
     * lookup misses; {@link com.example.evanscomputermod.sable.SableCompat}
     * searches plot chunks when sable is present.
     */
    private static net.minecraft.world.level.block.entity.BlockEntity
            resolveBlockEntity(Level level, BlockPos pos) {
        //? if <=1.21.1 {
        return com.example.evanscomputermod.sable.SableCompat.resolveBlockEntity(level, pos);
        //?} else
        /*return level.getBlockEntity(pos);*/
    }

    public static void handleTerminalDelta(TerminalDeltaPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            Level level = mc.level;
            if (level == null) return;

            TerminalDeltaPacket.ParsedDelta delta = packet.parse();

            if (delta.targetKind() == TerminalDeltaPacket.TARGET_SCREEN) {
                // Route to the anchor ScreenBlockEntity. The packet carries
                // the terminal's pos; find the anchor via any adjacent screen
                // that already knows its cluster membership.
                ScreenBlockEntity anchor = findAnchorForTerminal(level, packet.pos());
                if (anchor == null) return;

                TerminalDisplay display = anchor.clientDisplay;
                if (display == null) {
                    display = new TerminalDisplay(1, 1);
                    anchor.clientDisplay = display;
                }

                if (delta.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                    DeltaApplier.applyKeyframe(display, delta);
                } else {
                    DeltaApplier.applyDelta(display, delta);
                }

                ClientPacketDistributor.sendToServer(
                        new TerminalReadyPacket(packet.pos(), delta.targetKind(), delta.generation()));
                return;
            }

            if (resolveBlockEntity(level, packet.pos()) instanceof TerminalBlockEntity te) {
                TerminalDisplay display = te.getDisplay();

                if (delta.packetType() == TerminalDeltaPacket.PACKET_TYPE_KEYFRAME) {
                    DeltaApplier.applyKeyframe(display, delta);
                    te.clientHasKeyframe = true;
                } else if (te.clientHasKeyframe) {
                    DeltaApplier.applyDelta(display, delta);
                }
                // else: a delta for a screen this block entity never had (it loaded
                // after the server's shadow was set up); its keyframe is on the way.

                ClientPacketDistributor.sendToServer(
                        new TerminalReadyPacket(packet.pos(), delta.targetKind(), delta.generation()));
            }
        });
    }

    /** Ask the server for a terminal keyframe (generation 0 = "I have nothing"). */
    public static void requestKeyframe(BlockPos pos) {
        ClientPacketDistributor.sendToServer(
                new TerminalReadyPacket(pos, TerminalDeltaPacket.TARGET_TERMINAL, 0));
    }

    /** Walk the terminal's 6 neighbors to find a ScreenBlockEntity, then hop to its cluster anchor. */
    private static ScreenBlockEntity findAnchorForTerminal(Level level, BlockPos terminalPos) {
        for (net.minecraft.core.Direction dir : net.minecraft.core.Direction.values()) {
            BlockPos np = terminalPos.relative(dir);
            if (resolveBlockEntity(level, np) instanceof ScreenBlockEntity sbe) {
                BlockPos anchorPos = sbe.getClusterAnchor();
                if (anchorPos == null) continue;
                if (resolveBlockEntity(level, anchorPos) instanceof ScreenBlockEntity anchor) {
                    return anchor;
                }
            }
        }
        return null;
    }

    public static void handleOpenVisualEditor(OpenVisualEditorPacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            mc.setScreen(new VisualProgrammingScreen(packet.pos()));
        });
    }

    public static void handleProgramListResponse(ProgramListResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                List<String> programs = packet.fileListNewlineSeparated().isEmpty()
                        ? List.of()
                        : Arrays.asList(packet.fileListNewlineSeparated().split("\n"));
                vps.onProgramListReceived(programs);
            }
        });
    }

    public static void handleLoadProgramResponse(LoadProgramResponsePacket packet, IPayloadContext context) {
        context.enqueueWork(() -> {
            Minecraft mc = Minecraft.getInstance();
            if (mc.screen instanceof VisualProgrammingScreen vps) {
                vps.onProgramLoaded(packet.jsonContent());
            }
        });
    }
}
