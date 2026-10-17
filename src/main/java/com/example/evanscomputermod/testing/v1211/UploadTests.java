package com.example.evanscomputermod.testing.v1211;

//? if <=1.21.1 {
import com.example.evanscomputermod.testing.scenario.UploadScenarios;
import net.minecraft.gametest.framework.GameTest;
import net.minecraft.gametest.framework.GameTestHelper;
import net.neoforged.neoforge.gametest.GameTestHolder;
import net.neoforged.neoforge.gametest.PrefixGameTestTemplate;

/**
 * File upload from the player's real computer, namespace {@code ecm_upload}
 * (1.21.1): the {@link UploadScenarios}. Needs the evanscomputermod-upload
 * sidecar in the run (scripts/Test.ps1 -Upload).
 */
@GameTestHolder(UploadTests.NS)
@PrefixGameTestTemplate(false)
public final class UploadTests {
    static final String NS = "ecm_upload";

    /** upload on the console (and its controls), then Python's and Lua's shell.upload. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".upload_terminal")
    public static void upload_terminal(GameTestHelper h) {
        TestDriver.scenario(h, NS, UploadScenarios.ALL.get("upload_terminal"), true);
    }

    /** Files' Upload button on the desktop. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".upload_files_app")
    public static void upload_files_app(GameTestHelper h) {
        TestDriver.scenario(h, NS, UploadScenarios.ALL.get("upload_files_app"), true);
    }

    /** The ePhone's Files: ... then Upload. */
    @GameTest(template = TestDriver.STRUCTURE, timeoutTicks = TestDriver.BACKSTOP_TICKS, batch = NS + ".upload_phone")
    public static void upload_phone(GameTestHelper h) {
        TestDriver.scenario(h, NS, UploadScenarios.ALL.get("upload_phone"), true);
    }

    private UploadTests() {}
}
//?}
