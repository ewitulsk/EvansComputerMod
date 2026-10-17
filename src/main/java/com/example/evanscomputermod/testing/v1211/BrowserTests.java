package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.BrowserScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * The Browser (docs/gui/BROWSER.md), namespace {@code ecm_browser} (1.21.1): the
 * {@link BrowserScenarios}, player-built and operated, one batch each.
 */
@GameTestHolder(BrowserTests.NS)
@PrefixGameTestTemplate(false)
public final class BrowserTests {
    static final String NS = "ecm_browser";

    /** A site written on A, previewed as a file (flexbox), served with httpd and loaded from B; a link; controls: no stylesheet, 404. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".local_site")
    public static void browser_local_site(GameTestHelper h) {
        TestDriver.scenario(h, NS, BrowserScenarios.ALL.get("browser_local_site"), true);
    }

    /** A Tech Village house PC browses its village's site and follows a link over the ring; controls: no answer, 404. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".village")
    public static void browser_village(GameTestHelper h) {
        TestDriver.scenario(h, NS, BrowserScenarios.ALL.get("browser_village"), false);
    }

    /** An ePhone: no signal = an error page; over LTE a page loads, a drag scrolls it, a tap follows a link. */
    @GameTest(template = "gametest_cellular", timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".phone")
    public static void browser_phone(GameTestHelper h) {
        TestDriver.scenario(h, NS, BrowserScenarios.ALL.get("browser_phone"), true);
    }

    /** A Python CGI guestbook and a Lua web.serve guestbook: forms, POST, 303, cookies; control: an empty form refused. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".form")
    public static void browser_form(GameTestHelper h) {
        TestDriver.scenario(h, NS, BrowserScenarios.ALL.get("browser_form"), true);
    }

    /** A page script: a counter, fetch() of JSON; control: while (true) {} stopped by the budget, the page still answers. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".js")
    public static void browser_js(GameTestHelper h) {
        TestDriver.scenario(h, NS, BrowserScenarios.ALL.get("browser_js"), true);
    }

    /** At a Tech Village house PC (the ring booted, as DnsTests.ring): sites by name, the registrar's forms, a new .ecm name; controls: nosuch.ring, the same name twice. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".dns")
    public static void browser_dns(GameTestHelper h) {
        DnsTests.ring(h, NS, BrowserScenarios.ALL.get("browser_dns"));
    }

    /** At a Tech Village house PC: the Browser opens on the village's website (/etc/browser.conf), Home; controls: about:home, the player's own home wins. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".home")
    public static void browser_home(GameTestHelper h) {
        DnsTests.ring(h, NS, BrowserScenarios.ALL.get("browser_home"));
    }

    private BrowserTests() {}
}
//?}
