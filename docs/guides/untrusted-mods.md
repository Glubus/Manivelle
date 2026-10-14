# Untrusted Mods

Manivelle runs scripts in QuickJS on your thread. That is a sandbox for the language, not
a security boundary against a determined attacker: the process, its memory and every
host function you register belong to your application. This page says what a script
cannot do, what it can do to the others, and how to run code you do not trust.

## What a script cannot do

- It has no filesystem, network, process or clock API: it reaches the outside world only
  through the host functions you register, and each host function sees which script
  called it (`Caller`).
- It cannot import another script's modules, by name or by their internal ids, and
  `import()` is rejected: only your host modules and the modules of its own graph
  resolve.
- In a context of its own, nothing it does to the built-ins or the global object
  reaches another script.
- It cannot run forever: every operation has a time budget, and a stack and memory limit
  stop runaway recursion and allocation (see [Runtime Guarantees](runtime-guarantees.md)).

## What one script can do to the others

| Behaviour | Result |
| --- | --- |
| A handler or timer that never returns | Interrupted after `execution_timeout`; the scripts after it still run |
| The same script, again and again | Quarantined after `max_script_faults` faults (3 by default) |
| Allocating until memory runs out | Fails the script, counts as a fault; the pool is shared, so the others can fail to allocate while it holds memory |
| Patching `Array.prototype`, `Object`, `JSON`... | Reaches every script **of its context group**, and no script in a context of its own |
| A host function that blocks or panics | Not stopped: Rust host code is cooperative (see below) |

## Run untrusted code

1. **One context per untrusted script.** Use `load_script` and `load_project`, not
   `load_script_in`: a context group is a trust boundary, meant for scripts that trust
   each other (your own UI and HUD scripts, say).
2. **One `Engine` per untrusted mod when memory matters.** The memory limit
   (`EngineOptions::memory_limit_bytes`) is one pool per engine, and QuickJS has no
   per-script quota inside it. A mod that holds memory leaves less for the others of its
   engine. An `Engine` per mod gives each its own pool, its own time budget and its own
   quarantine list; an `Engine` costs one QuickJS runtime. Create it on the thread that
   will run it.
3. **Set the budgets for your frame, not for a batch job.** The default timeout is 5
   seconds, long enough to freeze a game. Pick something near what a frame can spare
   (`EngineOptions::execution_timeout`), and read `Engine::quarantined_scripts()` to tell
   the player which mod was disabled.
4. **Drop what you do not need.** `ScriptBuiltins` turns off `RegExp`, `Date`, `Proxy`,
   typed arrays, `WeakRef` and the web APIs per context; a smaller surface is less to
   reason about.
5. **Treat every host function as an API exposed to the internet.** Validate its
   input in Rust. Turn on `EngineOptions::contract_validation`
   (`ContractValidation::InputsAndOutputs`) and `unknown_field_validation`
   (`UnknownFieldValidation::Reject`) so the registered schema is enforced, never panic
   on a value a script chose, and never block: the time budget cannot preempt a Rust
   function that waits.
6. **Quarantine is not cleanup.** A quarantined script keeps its memory until you unload
   it; unload it (and tell the player) rather than leaving it loaded.

## What is out of scope

Manivelle does not protect against a malicious script exploiting a bug in QuickJS or in
one of your host functions, against side channels, or against native extensions: that
needs a process boundary, which no in-process engine provides. If a mod pack comes from
the internet and the host functions you give it are powerful (file writes, network),
run it in another process.
