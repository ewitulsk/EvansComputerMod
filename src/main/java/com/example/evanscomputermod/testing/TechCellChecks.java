package com.example.evanscomputermod.testing;

//? if <=1.21.1 {
import com.example.evanscomputermod.block.*;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.card.SimSecret;
import com.example.evanscomputermod.computer.*;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHardware;
import com.example.evanscomputermod.radio.cellular.CellAntennaBlock;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import com.example.evanscomputermod.radio.sdr.SdrBlockEntity;
import com.example.evanscomputermod.worldgen.CellSites;
import com.example.evanscomputermod.worldgen.TechVillageLocator;

import io.netty.buffer.ByteBufUtil;
import io.netty.buffer.Unpooled;
import net.minecraft.core.BlockPos;
import net.minecraft.core.Direction;
import net.minecraft.network.RegistryFriendlyByteBuf;
import net.minecraft.server.level.ServerLevel;
import net.minecraft.world.entity.decoration.ItemFrame;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.level.block.entity.ChestBlockEntity;
import net.minecraft.world.phys.AABB;

import java.nio.charset.StandardCharsets;
import java.nio.file.Files;
import java.util.*;

/**
 * Checks of the Tech Village carrier network (Cellular v2 phase 5) on generated blocks: the
 * Phone Store's contents against the core's database, SIM keys that never reach a client, the
 * store's cell site and the span sites' hardware. Shared by the {@code ecm_cellular} GameTest
 * and the {@code tech} client suite. Reading state to check it; nothing here causes a result.
 */
public final class TechCellChecks {
    private TechCellChecks() {}

    /** Phone Store positions in the Data Center template's coordinates (scripts/gen-tech-village.py). */
    public static final BlockPos CHEST = new BlockPos(17, 1, 2), STORE_SITE = new BlockPos(24, 1, 3), STORE_SDR = new BlockPos(24, 1, 2),
            STORE_ANTENNA = new BlockPos(27, 16, 2), TABLE = new BlockPos(17, 1, 8), STORE_DOOR = new BlockPos(20, 1, 11),
            STORE_INSIDE = new BlockPos(20, 1, 9);

    public static BlockPos store(TechVillageLocator.Visit visit, BlockPos local) {
        return TechVillageLocator.world(visit.datacenter(), local);
    }

    /** Null if the network encoding of {@code stack} (what a client receives) holds neither {@code key} nor its hex. */
    public static String keyOnTheWire(ServerLevel level, ItemStack stack, byte[] key) {
        RegistryFriendlyByteBuf buf = new RegistryFriendlyByteBuf(Unpooled.buffer(), level.registryAccess());
        ItemStack.OPTIONAL_STREAM_CODEC.encode(buf, stack);
        byte[] wire = ByteBufUtil.getBytes(buf);
        String text = new String(wire, StandardCharsets.ISO_8859_1).toLowerCase(Locale.ROOT);
        if (text.contains(CellPlan.hex(key))) return "the network encoding holds the key in hex";
        if (ByteBufUtil.indexOf(Unpooled.wrappedBuffer(key), Unpooled.wrappedBuffer(wire)) >= 0) return "the network encoding holds the raw key";
        ItemStack client = ItemStack.OPTIONAL_STREAM_CODEC.decode(new RegistryFriendlyByteBuf(Unpooled.wrappedBuffer(wire), level.registryAccess()));
        if (client.is(PhoneContent.EPHONE.get())) client = PhoneHardware.of(client).sim();
        SimSecret cs = SimCardItem.secret(client);
        if (cs != null && cs.known()) return "a client's copy knows the key";
        return null;
    }

    /** Null if {@code sim} is the provisioned SIM {@code m}: carrier, IMSI and key. */
    static String simIs(CellPlan plan, ItemStack sim, CellPlan.Sim m) {
        SimIdentity id = SimCardItem.identity(sim);
        SimSecret secret = SimCardItem.secret(sim);
        if (id == null) return "not a written SIM: " + sim;
        if (!id.carrier().equals(plan.carrier) || !id.imsiText().equals(m.imsi())) return "SIM " + id + " is not " + m.label() + " " + m.imsi();
        if (secret == null || !secret.known() || !Arrays.equals(secret.key(), m.key())) return "SIM " + m.label() + " does not hold its provisioned key";
        return null;
    }

    /** The core's database file holds exactly the planned sites and SIMs (each with its key). */
    public static String coreDb(ServerLevel level, WorldNetwork data) {
        CellPlan plan = data.cells(level);
        var path = ComputerStorage.path(level.getServer(), data.identity(level, plan.coreVillage, WorldNetwork.CORE)).resolve("etc/cored.db");
        try {
            String db = Files.readString(path);
            Map<String, String> subs = new HashMap<>(), sites = new HashMap<>();
            for (String line : db.split("\n")) {
                String[] w = line.trim().split("\\s+");
                if (w[0].equals("sub")) subs.put(w[1], w[2].substring(4));
                else if (w[0].equals("site")) sites.put(w[1], w[2]);
                else if (w[0].equals("carrier") && !w[1].equals(plan.carrier)) return "the database's carrier is " + w[1];
            }
            for (CellPlan.Sim m : plan.sims())
                if (!m.keyHex().equals(subs.get(m.imsi()))) return "the core's database lacks " + m.label() + " (" + m.imsi() + ") with its key";
            for (String id : plan.siteIds())
                if (!CellPlan.hex(plan.siteSecret(id)).equals(sites.get(id))) return "the core's database lacks site " + id + " with its credential";
            if (subs.size() != plan.sims().size() || sites.size() != plan.siteIds().size())
                return "the database has " + subs.size() + " subscribers and " + sites.size() + " sites, planned " + plan.sims().size() + "/"
                        + plan.siteIds().size();
            return null;
        } catch (Exception e) {
            return "no core database at " + path + ": " + e;
        }
    }

    /**
     * The Phone Store of village {@code n}: four ePhones lying in item frames on the display table
     * (LTE Modem, the village's phone SIMs), the chest's 27 SIMs, every key the core's database's
     * and none on the wire (frames, chest update tag); the cell site computer, its SDR and panel.
     */
    public static String store(ServerLevel level, WorldNetwork data, TechVillageLocator.Visit visit, int n, List<String> details) {
        CellPlan plan = data.cells(level);
        if (visit.datacenter() == null) return "village " + n + " has no data center / Phone Store";
        // The chest of SIMs.
        BlockPos chestPos = store(visit, CHEST);
        if (!(level.getBlockEntity(chestPos) instanceof ChestBlockEntity chest)) return "no SIM chest at " + chestPos.toShortString();
        for (int i = 1; i <= CellPlan.SIMS; i++) {
            ItemStack s = chest.getItem(i - 1);
            String why = simIs(plan, s, plan.sim(n, false, i));
            if (why != null) return "chest slot " + i + ": " + why;
            why = keyOnTheWire(level, s, plan.sim(n, false, i).key());
            if (why != null) return "chest SIM " + i + ": " + why;
        }
        if (chest.getUpdateTag(level.registryAccess()).contains("Items")) return "the chest's update tag carries its items";
        details.add("chest " + chestPos.toShortString() + ": " + CellPlan.SIMS + " SIMs " + plan.sim(n, false, 1).imsi() + "..");
        // The phones on display.
        BlockPos table = store(visit, TABLE);
        List<ItemFrame> frames = level.getEntitiesOfClass(ItemFrame.class, new AABB(table).inflate(5, 3, 5));
        Set<Integer> seen = new TreeSet<>();
        for (ItemFrame f : frames) {
            ItemStack phone = f.getItem();
            if (!phone.is(PhoneContent.EPHONE.get())) return "a display frame holds " + phone;
            if (f.getDirection() != Direction.UP) return "a display frame hangs " + f.getDirection() + ", not on the table";
            PhoneHardware hw = PhoneHardware.of(phone);
            boolean lte = hw.module1().is(CellularContent.modem(PhyProfile.LTE)) || hw.module2().is(CellularContent.modem(PhyProfile.LTE));
            if (!lte) return "a display phone has no LTE Modem: " + hw;
            SimIdentity id = SimCardItem.identity(hw.sim());
            if (id == null) return "a display phone has no SIM";
            int slot = -1;
            for (int i = 1; i <= CellPlan.PHONES; i++) if (plan.sim(n, true, i).imsi().equals(id.imsiText())) slot = i;
            if (slot < 0) return "a display phone's SIM " + id.imsiText() + " is not one of village " + n + "'s";
            String why = simIs(plan, hw.sim(), plan.sim(n, true, slot));
            if (why != null) return "display phone " + slot + ": " + why;
            why = keyOnTheWire(level, phone, plan.sim(n, true, slot).key());
            if (why != null) return "display phone " + slot + " in its frame: " + why;
            seen.add(slot);
        }
        if (seen.size() != CellPlan.PHONES) return "display phones " + seen + " of " + CellPlan.PHONES + " (" + frames.size() + " frames)";
        details.add("table " + table.toShortString() + ": phones " + seen);
        // The store's cell site.
        BlockPos sitePos = TechVillageLocator.role(level, visit, CellPlan.STORE_ROLE);
        if (sitePos == null || !(level.getBlockEntity(sitePos) instanceof TerminalBlockEntity site)
                || !site.getComputerId().equals(data.identity(level, n, CellPlan.STORE_ROLE)))
            return "no store cell site computer (" + sitePos + ")";
        try {
            var root = ComputerStorage.path(site);
            String conf = Files.readString(root.resolve("etc/celld-site.toml"));
            if (!conf.equals(plan.siteConfig(CellPlan.storeSiteId(n)))) return "the store site's /etc/celld-site.toml is not the planned one";
            if (!Files.readString(root.resolve("services.cfg")).contains("celld-site start --wait &")) return "celld-site is not enabled";
            if (!Files.readString(root.resolve("network.cfg")).contains(CellPlan.villageLan(n, CellPlan.STORE_HOST))) return "store site address";
        } catch (Exception e) {
            return "store site files: " + e;
        }
        String hw = siteHardware(level, store(visit, STORE_SDR), store(visit, STORE_ANTENNA));
        if (hw != null) return "store site: " + hw;
        Set<BlockPos> lan = TechServerChecks.cableComponent(level, sitePos.above());
        BlockPos web = TechVillageLocator.role(level, visit, WorldNetwork.WEB);
        if (!lan.contains(web.above())) return "the store site's cable does not reach the data center LAN (" + lan.size() + " blocks)";
        details.add("site " + sitePos.toShortString() + " on the data center LAN (" + lan.size() + " cable blocks)");
        return null;
    }

    /** The SDR's transmit chain reaches an LTE panel (coax from the SDR up to the antenna). */
    public static String siteHardware(ServerLevel level, BlockPos sdrPos, BlockPos antenna) {
        if (!(level.getBlockEntity(sdrPos) instanceof SdrBlockEntity sdr)) return "no Advanced SDR at " + sdrPos.toShortString();
        if (!(level.getBlockState(antenna).getBlock() instanceof CellAntennaBlock)) return "no LTE panel at " + antenna.toShortString();
        var panel = sdr.link().panel();
        // An SDR traces its chain on its own ticks; in a chunk that is loaded but not ticking, trace it the same way here.
        var traced = com.example.evanscomputermod.radio.amp.TransmitChain.resolve(level, sdrPos);
        if (panel == null && antenna.equals(traced.panel())) return null;
        if (panel == null) {
            var chain = traced;
            StringBuilder states = new StringBuilder();
            for (Direction d : Direction.values())
                states.append("\n  ").append(d).append(": ").append(level.getBlockState(sdrPos.relative(d)));
            return "the SDR's chain reaches no panel (coax from " + sdrPos.toShortString() + " to " + antenna.toShortString() + "): resolved "
                    + chain.panel() + " hops " + chain.hops() + "; SDR " + level.getBlockState(sdrPos) + states;
        }
        if (panel.tech() != PhyProfile.LTE) return "the panel is " + panel.tech();
        return null;
    }

    /** A span site as generated: its tap on the span, riser, computer (identity, files), SDR chain. */
    public static String spanSite(ServerLevel level, WorldNetwork data, CellPlan.SpanSite s, List<String> details) {
        var layout = CellSites.locate(level, s);
        if (layout == null) return s.id() + ": no computer in its column " + s.x() + " " + s.z();
        if (!(level.getBlockState(layout.panel()).getBlock() instanceof FiberPatchPanelBlock)) return s.id() + ": no tap panel at " + layout.panel();
        if (!(level.getBlockEntity(layout.computer()) instanceof TerminalBlockEntity be)
                || !be.getComputerId().equals(data.identity(level, s.village(), s.role())))
            return s.id() + ": wrong computer at " + layout.computer();
        // Panel -> riser -> the computer's face (eth1 from above, eth0 from below).
        BlockPos face = layout.eth() == 1 ? layout.computer().above() : layout.computer().below();
        Set<BlockPos> riser = TechServerChecks.cableComponent(level, layout.panel());
        if (!riser.contains(face) && !face.equals(layout.panel())) return s.id() + ": the riser does not reach eth" + layout.eth();
        try {
            var root = ComputerStorage.path(be);
            if (!Files.readString(root.resolve("etc/celld-site.toml")).equals(data.cells(level).siteConfig(s.id()))) return s.id() + ": config";
            if (!Files.readString(root.resolve("network.cfg")).startsWith("iface eth" + layout.eth() + " " + s.address() + "/26"))
                return s.id() + ": network.cfg is not eth" + layout.eth() + " " + s.address();
        } catch (Exception e) {
            return s.id() + ": files " + e;
        }
        String hw = siteHardware(level, layout.sdr(), layout.antenna());
        if (hw != null) return s.id() + ": " + hw;
        Direction facing = level.getBlockState(layout.antenna()).getValue(CellAntennaBlock.FACING);
        if (facing != Direction.from3DDataValue(s.facing())) return s.id() + ": the panel faces " + facing;
        details.add(s.id() + " computer " + layout.computer().toShortString() + " panel " + layout.panel().toShortString() + " eth" + layout.eth()
                + " antenna " + layout.antenna().toShortString() + " facing " + facing);
        return null;
    }

    // ------------------------------------------------------------ the Tech Village package repository

    /** Chat's app id: never installed by generation (the player installs it from the App Store). */
    public static final String CHAT_APP = "dev.ecm.chat";

    /** Null if the computer {@code id} has the world's repository (key pinned) in /etc/pkg/repos and no Chat installed. */
    public static String repoOn(ServerLevel level, VillageRepo repo, UUID id, String what) {
        var root = ComputerStorage.path(level.getServer(), id);
        try {
            var repos = root.resolve(VillageRepo.REPOS_FILE);
            if (!Files.exists(repos)) return what + " has no /etc/pkg/repos";
            if (!Files.readString(repos).equals(repo.reposFile())) return what + "'s /etc/pkg/repos is not the Tech Village repository with its key:\n"
                    + Files.readString(repos);
        } catch (Exception e) {
            return what + ": " + e;
        }
        if (Files.exists(root.resolve("apps/" + CHAT_APP))) return what + " has Chat installed already";
        return null;
    }

    /**
     * Every generated computer of village {@code n} ({@code visit.computers()}: ISP, Data Center,
     * Phone Store site, houses) and every display phone in its Phone Store: the Tech Village
     * repository in /etc/pkg/repos with the pinned key, and Chat not installed. The phones carry
     * their computer (fixed by the seed) from generation.
     */
    public static String repoConfigured(ServerLevel level, WorldNetwork data, TechVillageLocator.Visit visit, int n, List<String> details) {
        VillageRepo repo = data.repo(level);
        if (visit.computers().isEmpty()) return "village " + n + " has no computers";
        for (UUID id : visit.computers()) {
            String why = repoOn(level, repo, id, "computer " + id);
            if (why != null) return why;
        }
        BlockPos table = store(visit, TABLE);
        Set<UUID> phones = new HashSet<>();
        for (ItemFrame f : level.getEntitiesOfClass(ItemFrame.class, new AABB(table).inflate(5, 3, 5))) {
            UUID id = f.getItem().get(PhoneContent.PHONE_ID.get());
            if (id == null) return "a display phone has no computer yet";
            String why = repoOn(level, repo, id, "display phone " + id);
            if (why != null) return why;
            phones.add(id);
        }
        Set<UUID> planned = new HashSet<>();
        for (int slot = 1; slot <= CellPlan.PHONES; slot++) planned.add(data.identity(level, n, "store.phone" + slot));
        if (!phones.equals(planned)) return "display phones " + phones + ", planned " + planned;
        details.add(visit.computers().size() + " computers and " + phones.size() + " display phones of village " + n + ": "
                + VillageRepo.NAME + " " + repo.url() + " pinned " + repo.publicKey().substring(0, 16) + "..., no Chat");
        return null;
    }

    /**
     * The package server's repository as {@code pkg serve} published it (its index.json): format
     * ecm-repo-1, the world's publisher key, exactly one package, Chat, whose file is there with
     * that size and hash and whose signature verifies against the key the villages pin.
     */
    public static String repoIndex(ServerLevel level, WorldNetwork data, List<String> details) {
        VillageRepo repo = data.repo(level);
        var dir = ComputerStorage.path(level.getServer(), data.identity(level, repo.village(), WorldNetwork.REPO)).resolve(VillageRepo.DIR);
        try {
            var ix = com.google.gson.JsonParser.parseString(Files.readString(dir.resolve("index.json"))).getAsJsonObject();
            if (!"ecm-repo-1".equals(ix.get("format").getAsString())) return "index format " + ix.get("format");
            if (!ix.has("publisher_key") || !repo.publicKey().equals(ix.get("publisher_key").getAsString()))
                return "the index's publisher_key is not the world's repository key: " + ix.get("publisher_key");
            var pkgs = ix.getAsJsonArray("packages");
            if (pkgs.size() != 1) return "the repository lists " + pkgs.size() + " packages, not only Chat: " + pkgs;
            var e = pkgs.get(0).getAsJsonObject();
            String id = e.get("id").getAsString(), version = e.get("version").getAsString(), file = e.get("file").getAsString();
            if (!id.equals(CHAT_APP)) return "the repository's package is " + id;
            byte[] bytes = Files.readAllBytes(dir.resolve(file));
            var p = VillageRepo.read(file, bytes);
            if (bytes.length != e.get("size").getAsLong() || !p.sha256().equals(e.get("sha256").getAsString())) return file + ": size or sha256 differ from the index";
            if (!e.has("signature")) return "Chat is not signed";
            if (!VillageRepo.verify(repo.publicKey(), VillageRepo.signedMessage(id, version, p.sha256()), e.get("signature").getAsString()))
                return "Chat's signature does not verify with the pinned key";
            details.add("index.json: " + id + " " + version + " (" + bytes.length + " bytes, signed, verifies with " + repo.publicKey().substring(0, 16)
                    + "...)");
            return null;
        } catch (Exception ex) {
            return "no valid index.json in " + dir + ": " + ex;
        }
    }
}
//?}