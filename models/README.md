# Tech infrastructure models

These editable Java Block/Item projects were authored and painted through the
connected Blockbench MCP. Each `.bbmodel` embeds its texture so it can be opened
without this checkout's absolute texture paths. Preview PNGs are native
Blockbench renders; game screenshots are retained in test receipts.

| Project | Design |
|---|---|
| `fiber_span.bbmodel` | Sealed center coupler, identification band and six separate jacketed arms |
| `fiber_patch_panel.bbmodel` | Powder-coated rack case, ears/screws, eight duplex LC sockets, labels, LED and vent |
| `always_on_module.bbmodel` | Green PCB, integrated circuit, traces, gold contacts and power indicator |

Export after editing with:

```powershell
py -3 scripts/gen-tech-assets.py
```

The exporter extracts the embedded PNGs, converts the project's UV coordinates
to Minecraft's 16-unit convention, and preserves cube geometry. Fiber cube names
start with `center`, `north`, `south`, `west`, `east`, `down` or `up`; the exporter
uses those names to produce multipart models. Preserve these prefixes when editing.
The patch panel gets four facing variants. Item models use the complete projects.

Validate in Minecraft with `scripts/Test.ps1 -Area tech-models -ClientChecks
-NoStage`, then inspect the paired screenshot receipt. `/ecm scenario spawn
router_fiber manual` provides a playable physical model and connection test.
