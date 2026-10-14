# Untrusted Mods

Manivelle runs scripts in QuickJS on your thread. That is a sandbox for the language, not
a security boundary against a determined attacker: the process, its memory and every
host function you register belong to your application. This page says what a script
cannot do, what it can do to the others, and how to run code you do not trust.

## What a script cannot do

- It has no filesystem, network or process API: it reaches the outside world only
  through the host functions you register, and each host function sees which script
  called it (`Caller`). It does have clocks (`Date.now()`, `performance.now()`) unless
  you turn off the `date` and `web` built-ins, and it can write to the `console` sink
  (see below).
- It cannot import another script's modules, by name or by their internal ids: the
  module resolver only returns your host modules and the modules of the script's own
  graph, whether the `import` is written in the source or built at run time (`eval`
  and `new Function` stay available, since modules are compiled with them, but what they
  import goes through the same resolver).
- In a context of its own, nothing it does to the built-ins or the global object changes
  the semantics another script sees.
- It cannot run forever: every operation has a time budget, and a stack and memory limit
  stop runaway recursion and allocation of script values (see
  [Runtime Guarantees](runtime-guarantees.md)). `EngineOptions` refuses a stack or memory
  limit of 0, which QuickJS reads as "no limit".

## What one script can do to the others

| Behaviour | Result |
| --- | --- |
| A handler, timer or Promise job that never returns | Interrupted after `execution_timeout`, charged to the script that queued it; the scripts after it still run |
| The same script, again and again | Quarantined after `max_script_faults` faults (3 by default) |
| Allocating until the pool is full | The pool is one per engine, so the others can fail to allocate while it holds memory. Only the first script to fail for lack of memory in an operation is counted as faulting, so the victims are not all quarantined, but the one counted may be a victim if the hog catches the error |
| Patching `Array.prototype`, `Object`, `JSON`... | Changes every script **of its context group**, and no script in a context of its own |
| Reading or rewriting a neighbour's data and handlers in a context group | Possible: a group shares one global object. A neighbour can also cause a script to be quarantined |
| Flooding the `console` | The sink receives every message untouched and at any size: a script can forge log lines and terminal escapes, and a slow sink slows the host |
| Memory held outside QuickJS | `NativeBytes` your host functions hand out lives in Rust memory the pool does not count; `TextEncoder` output is counted |
| A host function that blocks or panics | Not stopped: Rust host code is cooperative (see below) |

## Run untrusted code

1. **One context per untrusted script.** Use `load_script` and `load_project`, not
   `load_script_in`: a context group is a trust boundary, meant for scripts that trust
   each other (your own UI and HUD scripts, say).
2. **One `Engine` per untrusted mod.** The memory limit
   (`EngineOptions::memory_limit_bytes`) is one pool per engine, and QuickJS has no
   per-script quota inside it. A mod that holds memory leaves less for the others of its
   engine, and quarantine cannot tell the hog from its victims reliably. An `Engine` per
   mod gives each its own pool, its own time budget and its own quarantine list; an
   `Engine` costs one QuickJS runtime. Create it on the thread that will run it, and give
   that thread a stack larger than `max_stack_size_bytes`.
3. **Set the budgets for your frame, not for a batch job.** The default timeout is 5
   seconds, long enough to freeze a game. Pick something near what a frame can spare
   (`EngineOptions::execution_timeout`), and read `Engine::quarantined_scripts()` to tell
   the player which mod was disabled.
4. **Drop what you do not need.** `ScriptBuiltins` turns off `RegExp`, `Date`, `Proxy`,
   typed arrays, `WeakRef` and the web APIs per context; a smaller surface is less to
   reason about. `eval` and `new Function` cannot be turned off.
5. **Treat every host function as an API exposed to the internet.** Validate its
   input in Rust. Turn on `EngineOptions::contract_validation`
   (`ContractValidation::InputsAndOutputs`) and `unknown_field_validation`
   (`UnknownFieldValidation::Reject`) so the registered schema is enforced. Values that
   cross as JSON may nest 128 levels deep (a cyclic value is refused); a type of yours
   that is recursive (`Box<Node>`) decodes as deep as the script's value goes, so bound
   it yourself. Never panic on a value a script chose, and never block: the time budget
   cannot preempt a Rust function that waits.
6. **Quarantine is not cleanup.** A quarantined script keeps its memory until you unload
   it; unload it (and tell the player) rather than leaving it loaded. A script can also
   make its own reload fail through a throwing `ctx.hot.save`; `unload_script` always
   works.

## What is out of scope

Manivelle does not protect against a malicious script exploiting a bug in QuickJS or in
one of your host functions, against side channels, or against native extensions: that
needs a process boundary, which no in-process engine provides. If a mod pack comes from
the internet and the host functions you give it are powerful (file writes, network),
run it in another process.
