package com.example.evanscomputermod.testing.scenario;

//? if <=1.21.1 {
import com.example.evanscomputermod.api.IComputerHost;
import com.example.evanscomputermod.computer.ComputerInstance;
import com.example.evanscomputermod.computer.ui.Compositor;
import com.example.evanscomputermod.computer.ui.NodeTree;
import com.example.evanscomputermod.computer.ui.PhoneUi;
import com.example.evanscomputermod.phone.PhoneConfig;
import com.example.evanscomputermod.phone.PhoneContent;
import com.example.evanscomputermod.phone.PhoneHost;
import com.example.evanscomputermod.phone.PhoneItemData;
import com.example.evanscomputermod.phone.PhoneMenu;
import com.example.evanscomputermod.phone.PhoneServer;
import com.example.evanscomputermod.phone.PhoneState;
import com.example.evanscomputermod.radio.cellular.CellularContent;
import com.example.evanscomputermod.radio.cellular.SimCardItem;
import com.example.evanscomputermod.card.SimIdentity;
import com.example.evanscomputermod.radio.framed.PhyProfile;
import net.minecraft.world.entity.player.Inventory;
import net.minecraft.world.item.ItemStack;
import net.minecraft.world.phys.Vec3;

import java.util.List;
import java.util.UUID;

/**
 * One person with an ePhone in a scenario with several phones (telephony): the scenario
 * player (the first) or a second scenario player, and the phone they hold. Everything goes
 * through the same packet handlers as {@link PhoneScenarios} and {@link PhoneGuiScenarios},
 * for any holder.
 */
final class Handset {
    final String name;
    /** Null for the scenario's own player ({@link ScenarioRun#player()}). */
    final ScenarioPlayer other;
    UUID phone;
    String imsi = "", number = "";
    long mark;
    String memo = "";
    /** The last shell line typed on the Terminal ({@link TelephonyScenarios#run}). */
    String typed = "";

    Handset(String name, ScenarioPlayer other) {
        this.name = name;
        this.other = other;
    }

    ScenarioPlayer sp(ScenarioRun r) {
        return other == null ? r.player() : other;
    }

    Inventory inv(ScenarioRun r) {
        return sp(r).entity().getInventory();
    }

    /** A new ePhone in hotbar slot 1 (selected), right-clicked once in the air: its first use gives it its computer. */
    void givePhone(ScenarioRun r, Vec3 at) {
        if (other == null) {
            PhoneScenarios.givePhone(r);
            phone = PhoneScenarios.st(r).phone;
        } else {
            PhoneScenarios.standAt(other, r, at);
            PhoneServer.track(other.entity());
            ItemStack p = new ItemStack(PhoneContent.EPHONE.get());
            other.useInAir(p, false);
            phone = PhoneItemData.id(p);
            if (phone == null) throw new IllegalStateException("first use gave " + name + "'s phone no computer");
            other.entity().getInventory().setItem(0, p);
            other.entity().getInventory().selected = 0;
        }
        // The battery drains per server tick, and a GameTest server ticks unthrottled while the phone runs in real time.
        PhoneConfig.override("battery.enabled", false);
        r.say("§7  " + name + " used a new ePhone (computer " + phone.toString().substring(0, 8) + ") and holds it");
    }

    void standAt(ScenarioRun r, Vec3 at) {
        PhoneScenarios.standAt(sp(r), r, at);
    }

    PhoneHost host(ScenarioRun r) {
        return phone == null ? null : PhoneServer.find(r.level().getServer(), phone);
    }

    ComputerInstance computer(ScenarioRun r) {
        PhoneHost h = host(r);
        return h == null ? null : h.instance();
    }

    Compositor comp(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null || c.getUiManager() == null ? null : c.getUiManager().compositor();
    }

    PhoneUi ui(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null || c.getUiManager() == null ? null : c.getUiManager().phone();
    }

    /** {@code /dev/uictl phone}: the status bar's source. */
    String status(ScenarioRun r) {
        ComputerInstance c = computer(r);
        return c == null || c.getUiManager() == null ? "" : c.getUiManager().control("phone");
    }

    String statusHas(ScenarioRun r, String want) {
        String s = status(r);
        return s.contains(want) ? null : name + "'s uictl phone:\n" + s;
    }

    /** Null once the phone is awake in its shell with the home grid up. */
    String guiBooted(ScenarioRun r) {
        PhoneHost h = host(r);
        if (h == null || h.state() != PhoneState.AWAKE || !h.isAttached()) return name + "'s phone not awake";
        Compositor c = comp(r);
        if (ui(r) == null || c == null || !c.shellConnected()) return name + "'s phone is booting to its shell";
        return c.layerTexts("home").contains("Notes") ? null : "home grid " + c.layerTexts("home");
    }

    // ------------------------------------------------------------ hands and slots

    /** Move the phone to inventory slot {@code to} (0-8 hotbar, 9-35 pocket, 40 off hand), swapping with what is there. */
    void movePhone(ScenarioRun r, int to) {
        Inventory inv = inv(r);
        int from = -1;
        for (int i = 0; i < inv.getContainerSize(); i++) if (phone.equals(PhoneItemData.id(inv.getItem(i)))) from = i;
        if (from < 0) throw new IllegalStateException(name + " has no phone");
        ItemStack s = inv.getItem(from);
        inv.setItem(from, inv.getItem(to));
        inv.setItem(to, s);
    }

    /** The phone in the main hand (hotbar slot 1, selected). */
    void phoneToHand(ScenarioRun r) {
        Inventory inv = inv(r);
        if (!inv.getItem(0).isEmpty() && !phone.equals(PhoneItemData.id(inv.getItem(0))))
            for (int i = 9; i < 36; i++) if (inv.getItem(i).isEmpty()) {
                inv.setItem(i, inv.getItem(0));
                inv.setItem(0, ItemStack.EMPTY);
                break;
            }
        movePhone(r, 0);
        inv.selected = 0;
    }

    /** The phone in the off hand (held: awake), the main hand the empty slot 9. */
    void phoneToOffHand(ScenarioRun r) {
        movePhone(r, 40);
        inv(r).selected = 8;
    }

    static int slotOfSim(Inventory inv, String imsi) {
        for (int i = 0; i < 36; i++) {
            SimIdentity id = SimCardItem.identity(inv.getItem(i));
            if (id != null && id.imsiText().equals(imsi)) return i;
        }
        return -1;
    }

    static int slotOfModem(Inventory inv) {
        for (int i = 0; i < 36; i++) if (inv.getItem(i).is(CellularContent.modem(PhyProfile.LTE))) return i;
        return -1;
    }

    /** Sneak-right-click the phone in hand (its slot screen) and shift-click an LTE Modem and this person's SIM into it. */
    void fitModemAndSim(ScenarioRun r) {
        ScenarioPlayer p = sp(r);
        p.useHeld(true);
        if (!(p.entity().containerMenu instanceof PhoneMenu menu)) {
            r.fail(name + ": sneak-right-clicking the phone opened no slot screen");
            return;
        }
        int m = slotOfModem(inv(r)), s = slotOfSim(inv(r), imsi);
        if (m < 0 || s < 0) {
            r.fail(name + " has no LTE Modem (" + m + ") or SIM " + imsi + " (" + s + ")");
            return;
        }
        p.shiftClick(CellPhoneScenarios.menuIndex(m));
        p.shiftClick(CellPhoneScenarios.menuIndex(s));
        boolean modemIn = menu.getSlot(PhoneMenu.MODULE_1).getItem().is(CellularContent.modem(PhyProfile.LTE))
                || menu.getSlot(PhoneMenu.MODULE_2).getItem().is(CellularContent.modem(PhyProfile.LTE));
        boolean simIn = SimCardItem.isIssued(menu.getSlot(PhoneMenu.SIM).getItem());
        p.closeScreen();
        if (!modemIn || !simIn) r.fail(name + "'s slot screen: modem in " + modemIn + ", SIM in " + simIn);
        else r.say("§7  " + name + ": LTE Modem in a module slot, the SIM (" + number + ") in the SIM tray");
    }

    /** Sneak-right-click the phone and shift-click the SIM out of the tray (into the inventory). */
    void simOut(ScenarioRun r) {
        ScenarioPlayer p = sp(r);
        p.useHeld(true);
        if (!(p.entity().containerMenu instanceof PhoneMenu)) {
            r.fail(name + ": no slot screen");
            return;
        }
        p.shiftClick(PhoneMenu.SIM);
        p.closeScreen();
        if (slotOfSim(inv(r), imsi) < 0) r.fail(name + ": the SIM didn't come out");
    }

    /** The SIM back into the tray (slot screen, shift-click it from the inventory). */
    void simIn(ScenarioRun r) {
        ScenarioPlayer p = sp(r);
        p.useHeld(true);
        if (!(p.entity().containerMenu instanceof PhoneMenu menu)) {
            r.fail(name + ": no slot screen");
            return;
        }
        int s = slotOfSim(inv(r), imsi);
        if (s < 0) {
            r.fail(name + " has no SIM " + imsi);
            p.closeScreen();
            return;
        }
        p.shiftClick(CellPhoneScenarios.menuIndex(s));
        boolean simIn = SimCardItem.isIssued(menu.getSlot(PhoneMenu.SIM).getItem());
        p.closeScreen();
        if (!simIn) r.fail(name + ": the SIM didn't go into the tray");
    }

    // ------------------------------------------------------------ input

    /** Type on the open phone as its holder; a newline is Enter. */
    boolean type(ScenarioRun r, String text) {
        ScenarioPlayer p = sp(r);
        boolean ok = true;
        StringBuilder seg = new StringBuilder();
        for (char ch : text.toCharArray()) {
            if (ch == '\n' || ch == '\u0014') {
                if (!seg.isEmpty()) ok &= p.type(phone, IComputerHost.DISPLAY_OWN, seg.toString());
                seg.setLength(0);
                ok &= ch == '\n' ? p.enter(phone, IComputerHost.DISPLAY_OWN) : p.ctrl(phone, IComputerHost.DISPLAY_OWN, 't');
            } else {
                seg.append(ch);
            }
        }
        if (!seg.isEmpty()) ok &= p.type(phone, IComputerHost.DISPLAY_OWN, seg.toString());
        return ok;
    }

    boolean tapAt(ScenarioRun r, int[] at) {
        return at != null && sp(r).tap(phone, IComputerHost.DISPLAY_OWN, at[0], at[1]);
    }

    /** Tap the text {@code text} in the window titled {@code title}: null when tapped, else why not (for an await). */
    String tap(ScenarioRun r, String title, String text) {
        Compositor c = comp(r);
        if (c == null) return name + ": no phone GUI";
        int[] at = c.findText(title, text);
        if (at == null) return name + ": \"" + text + "\" not shown in " + title + " yet: " + texts(r, title);
        return tapAt(r, at) ? null : name + "'s phone refused the touch";
    }

    /** Tap {@code label} on the shell's {@code layer} (home grid, overlay, notifications). */
    String tapLayer(ScenarioRun r, String layer, String label, boolean contains) {
        Compositor c = comp(r);
        if (c == null) return name + ": no phone GUI";
        int[] at = c.locateLayerText(layer, label, contains);
        if (at == null) return name + ": '" + label + "' not on the " + layer + " layer: " + c.layerTexts(layer);
        return tapAt(r, at) ? null : name + "'s phone refused the touch";
    }

    void altKey(ScenarioRun r, int hid) {
        if (!sp(r).chord(phone, IComputerHost.DISPLAY_OWN, hid, PhoneGuiScenarios.ALT)) r.fail(name + "'s phone refused the keys");
    }

    // ------------------------------------------------------------ what it shows

    Compositor.WindowInfo onScreen(ScenarioRun r, String appId) {
        Compositor c = comp(r);
        if (c == null) return null;
        for (Compositor.WindowInfo w : c.windows())
            if (w.appId().equals(appId) && w.focused() && w.visible() && w.mapped() && w.y() == 16) return w;
        return null;
    }

    /** Every text in the (mapped) window titled {@code title}. */
    List<String> texts(ScenarioRun r, String title) {
        Compositor c = comp(r);
        if (c == null) return List.of();
        for (Compositor.WindowInfo w : c.windows()) if (w.title().equals(title) && w.mapped()) return c.textsOf(w.id());
        return List.of();
    }

    /** Null when a text in window {@code title} contains {@code want}. */
    String shows(ScenarioRun r, String title, String want) {
        List<String> t = texts(r, title);
        return t.stream().anyMatch(s -> s.contains(want)) ? null : name + "'s " + title + " shows " + t;
    }

    List<Compositor.PostedNotification> notifications(ScenarioRun r) {
        Compositor c = comp(r);
        return c == null ? List.of() : c.notifications();
    }

    /** The Terminal app's text grid (as {@link PhoneScenarios#screen}). */
    String screen(ScenarioRun r) {
        Compositor comp = comp(r);
        if (comp == null) return "";
        Compositor.WindowInfo term = null;
        for (Compositor.WindowInfo w : comp.windows()) if (w.appId().equals(PhoneScenarios.TERMINAL) && w.mapped()) term = w;
        if (term == null) return "";
        List<NodeTree.PlacedText> texts = comp.placedTextsOf(term.id());
        if (texts.isEmpty()) return "";
        int x0 = Integer.MAX_VALUE, y0 = Integer.MAX_VALUE;
        for (NodeTree.PlacedText t : texts) {
            x0 = Math.min(x0, t.x());
            y0 = Math.min(y0, t.y());
        }
        java.util.TreeMap<Integer, StringBuilder> rows = new java.util.TreeMap<>();
        for (NodeTree.PlacedText t : texts) {
            StringBuilder line = rows.computeIfAbsent((t.y() - y0) / 16, k -> new StringBuilder());
            int col = (t.x() - x0) / 8;
            while (line.length() < col) line.append(' ');
            line.replace(col, Math.min(line.length(), col + t.text().length()), t.text());
        }
        StringBuilder out = new StringBuilder();
        for (int y = 0; y <= rows.lastKey(); y++) {
            StringBuilder line = rows.get(y);
            out.append(line == null ? "" : line).append('\n');
        }
        return out.toString();
    }

    /** The phone's log file (diagnosis). */
    String log(ScenarioRun r, String file, int tail) {
        try {
            ComputerInstance c = computer(r);
            String t = java.nio.file.Files.readString(c.storagePath().resolve(file));
            return t.substring(Math.max(0, t.length() - tail));
        } catch (Exception e) {
            return "(no " + file + ": " + e + ")";
        }
    }

    // ------------------------------------------------------------ cleanup

    /** Close the phone's computer (cleanup). */
    void forget(ScenarioRun r) {
        if (other != null) PhoneServer.untrack(other.entity());
        if (phone == null || other == null) return;
        Inventory inv = other.entity().getInventory();
        for (int i = 0; i < inv.getContainerSize(); i++) if (phone.equals(PhoneItemData.id(inv.getItem(i)))) inv.setItem(i, ItemStack.EMPTY);
        PhoneHost h = PhoneServer.find(r.level().getServer(), phone);
        if (h != null && h.instance() != null) {
            h.instance().close();
            com.example.evanscomputermod.computer.ComputerHost.get(r.level().getServer(), phone).forget();
        }
    }
}
//?}
