# Mod Integration API — Brainstorm

## The Core Problem

Adding a host function today requires changes at **4 layers**: Java (`TerminalWasmHost`) → Rust extern decl → Python module → Visual blocks JSON. The Rust/WASM layer requires **recompilation**, which is a dealbreaker for third-party mod authors. They can't ship new `.wasm` binaries.

---

## Approach 1: "JSON-RPC Bridge" — Bypass the WASM boundary entirely

Instead of adding new WASM host functions per-mod, provide a **single generic host function** like `mod_call(mod_id, method, args_json) -> result_json`. Third-party mods register Java-side handlers, and the existing WASM binary routes calls through without needing recompilation.

**Java side:** A registry where mods register handlers:
```java
ComputerAPI.registerFunction("create-golem:summon", (computer, args) -> {
    // do minecraft stuff
    return JsonResult.success(...);
});
```

**Python side:** A generic `mods` module already baked into the WASM binary:
```python
import mods
mods.call("create-golem", "summon", {"type": "iron"})
# or with auto-discovery:
golem = mods.wrap("create-golem")
golem.summon(type="iron")
```

**Pros:** Zero WASM recompilation. Dead simple for mod authors — just register Java callbacks. The `mods.wrap()` pattern mirrors the existing `peripheral.wrap()`.

**Cons:** Everything is stringly-typed JSON. No compile-time safety. Slight overhead from serialization.

---

## Approach 2: "Capability-Based Embedding" — Computers as a capability, not a block

Right now the computer is a `TerminalBlockEntity`. To let mods embed computers in mobs/items/vehicles, extract the computer logic into a **reusable component** that can be attached to anything.

**Core idea:** Pull `TerminalBlockEntity`'s WASM host, terminal buffer, file system, and worker thread into a `ComputerInstance` class. Then provide attachment points:

- `ComputerBlockAttachment` — current terminal block behavior
- `ComputerEntityAttachment` — for mobs (turtles, drones)
- `ComputerItemAttachment` — for handheld computers

The attachment provides the **context** (position in world, inventory access, movement API), and the `ComputerInstance` provides the runtime. Mod authors create a new attachment type and register context-specific functions:

```java
public class DroneAttachment extends ComputerAttachment {
    @ComputerFunction("drone.move_forward")
    public int moveForward(JsonObject args) {
        this.entity.moveForward(1);
        return 0;
    }
}
```

This pairs naturally with Approach 1 — the attachment registers its functions into the JSON-RPC bridge.

---

## Approach 3: "Annotation-Driven Auto-Registration"

Use Java annotations to make exposing functions to Python completely automatic:

```java
@ComputerModule("golem")
public class GolemComputer {

    @ComputerFunction(description = "Summon a golem")
    @Param(name = "type", type = "string", description = "Golem type")
    @Returns(type = "boolean", description = "Success")
    public boolean summon(ComputerContext ctx, String type) {
        // ...
    }
}
```

At mod load time, scan for `@ComputerModule` annotations, introspect the methods, and:
1. Auto-register them in the Java-side call router
2. Auto-generate Python wrapper code (string-built at runtime, injected into the Python VM's import system)
3. Auto-generate visual programming blocks (blocks are already loaded from JSON — generate entries dynamically)

The Python user just does:
```python
import golem
golem.summon("iron")
```

**This is the "it just works" approach.** Mod author annotates a class, and it appears in Python and the visual editor automatically.

---

## Approach 4: "Peripheral-Style Discovery" — Extend what already exists

The existing peripheral system discovers CC:Tweaked peripherals by scanning adjacent blocks. This could be generalized into a **native peripheral API** that doesn't depend on CC:Tweaked:

```java
public class GolemPeripheral implements IComputerPeripheral {
    @Override
    public String getType() { return "golem_controller"; }

    @Override
    public Map<String, PeripheralMethod> getMethods() {
        return Map.of(
            "summon", args -> { /* ... */ },
            "recall", args -> { /* ... */ }
        );
    }
}
```

This leverages the existing `peripheral.call()` WASM host function — no WASM changes needed. Mod authors just implement an interface and register it via NeoForge capabilities. Python code uses the existing peripheral module:

```python
import peripheral
ctrl = peripheral.wrap("golem_controller_0")
ctrl.call("summon", '{"type": "iron"}')
```

**Pros:** Minimal new code — extends existing proven pattern. Already works through WASM without recompilation.

**Cons:** The `peripheral.call()` API is awkward (JSON strings). Would want to layer Approach 3's auto-generation on top for nice Python wrappers.

---

## Approach 5: Hybrid (Recommended)

Combine the best parts of all approaches:

1. **Approach 2** — Extract `ComputerInstance` from the block entity so computers can be embedded anywhere
2. **Approach 4** — Generalize the peripheral system as the primary extension point (it already works through WASM!)
3. **Approach 3's auto-generation** — On top of the peripheral system, auto-generate nice Python modules and visual blocks from annotated Java classes
4. **Approach 1's generic bridge** — As the underlying transport for non-peripheral functions (global mod APIs that aren't tied to a block)

The mod author experience becomes:

```java
// Register a block-adjacent peripheral (like CC:Tweaked)
@ComputerPeripheral("golem_controller")
public class GolemController { ... }

// Register global functions (available to all computers)
@ComputerModule("weather")
public class WeatherAPI { ... }

// Embed a computer in a mob
ComputerInstance computer = ComputerAPI.createComputer(mob);
computer.registerAttachment(new MobMovementAPI(mob));
```

And the Python user gets:
```python
import peripheral  # block-adjacent stuff
import weather     # global mod APIs
import movement    # context-specific (only on mob computers)
```

---

## Key Design Decision: The WASM Recompilation Problem

The single biggest architectural question is: **how do you avoid requiring WASM recompilation?**

All approaches above solve it the same way — route through a **generic bridge function** already compiled into the WASM binary. The debate is just about what the Java-side registration API looks like and how nice the Python-side auto-generated wrappers are.

The existing `peripheral_call(name, method, args_json, result_json)` host function is essentially already this bridge — it's just scoped to adjacent blocks. Generalizing it to a `mod_call()` equivalent is probably the lowest-effort high-impact change.
