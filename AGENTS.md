# EvansComputerMod development

Follow the targeted testing workflow in `TESTING.md` and `../ModTesting.md`.
Build a coherent change before running its tests, retain failure receipts, and
report executed counts. Tests and test worlds stay inside this repository.

Every major gameplay feature must include usable `/ecm scenario spawn` scenarios.
Register them in the existing scenario command, provide `auto`, `manual`, and
`fast` modes where applicable, and make `/ecm scenario commands` explain the
setup, commands, expected results, and meaningful failure/control cases. Reuse
the same definitions in targeted GameTests rather than maintaining separate
implementations. A feature is unfinished if its scenarios are absent, silently
skip its important behavior, or were written without being run.

World-generation tests must exercise normal chunk generation in a fresh world,
not only `/place structure`. Client-visible block changes require a real
Minecraft rendering check with a screenshot and paired server/client assertions.
Use an isolated scripted server and hidden client, as described in ModTesting;
never automate the user's desktop, game profile, or saves for these tests.

Use the connected Blockbench MCP for new or substantially revised block models.
Keep editable `.bbmodel` sources, export game assets, inspect Blockbench previews,
and then inspect the Minecraft render. Neighbor-dependent models must have
tested placement, connection, removal, and reload behavior. Do not substitute
vanilla placeholder textures for a finished visual design.
