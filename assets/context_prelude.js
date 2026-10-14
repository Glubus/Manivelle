// The script environment, built once per script, in a context of its own or in one
// shared with other scripts (a context group). The engine compiles this module once and
// loads its bytecode into every context, so a context does not pay for a parse and a
// compile.
//
// `makeEnv(hooks)` returns the script's `ctx`, `console` and timers, and the hooks the
// engine drives. `hooks` holds what the engine bound to the script: `native` (its host
// functions), `handlers` (keeps the engine's copy of each event's handler list), `console`
// (the host sink), `now` and `schedule` (the engine clock and the script's next due
// time) and, on a reload, `hotData`, the previous version's saved state.
// `installGlobals(env)` also exposes them as globals, for a script in a context of its
// own; a script in a group imports them from `manivelle:env` instead.

export function makeEnv(hooks) {
  // Hot reload: `ctx.hot.data` is what the previous version's `save` returned. The
  // engine only calls the `save` and `dispose` hooks when it replaces or unloads the
  // script.
  function hotReload(data) {
    let save = () => undefined;
    const disposers = [];
    const expectFunction = (name, fn) => {
      if (typeof fn !== "function") {
        throw new TypeError(`ctx.hot.${name} expects a function`);
      }
    };
    return {
      api: {
        data,
        save(fn) {
          expectFunction("save", fn);
          save = fn;
        },
        dispose(fn) {
          expectFunction("dispose", fn);
          disposers.push(fn);
        },
      },
      hooks: {
        save: () => save(),
        // Every disposer runs even after one throws; the first error is rethrown.
        dispose: () => {
          let failed = false;
          let failure;
          for (const disposer of disposers) {
            try {
              disposer();
            } catch (error) {
              if (!failed) {
                failed = true;
                failure = error;
              }
            }
          }
          if (failed) {
            throw failure;
          }
        },
      },
    };
  }

  // Event handlers: `ctx.on(event, handler)` / `ctx.off(event, handler)`. Every change
  // hands the event's list to the engine through `setHandlers` (`undefined` once it
  // has no handler), which keeps a snapshot of its functions: delivering an event
  // looks nothing up by name, skips contexts that do not listen to it, and a
  // delivery in progress keeps the snapshot it took.
  const setHandlers = hooks.handlers;
  const handlers = new Map();
  const expectHandler = (method, eventName, handler) => {
    if (typeof eventName !== "string") {
      throw new TypeError(`${method} expects a string event name`);
    }
    if (typeof handler !== "function") {
      throw new TypeError(`${method} expects a function handler`);
    }
  };
  const on = (eventName, handler) => {
    expectHandler("ctx.on", eventName, handler);
    let list = handlers.get(eventName);
    if (list === undefined) {
      list = [];
      handlers.set(eventName, list);
    }
    list.push(handler);
    setHandlers(eventName, list);
  };
  const off = (eventName, handler) => {
    expectHandler("ctx.off", eventName, handler);
    const list = handlers.get(eventName);
    const index = list === undefined ? -1 : list.indexOf(handler);
    if (index < 0) {
      return;
    }
    list.splice(index, 1);
    if (list.length === 0) {
      handlers.delete(eventName);
      setHandlers(eventName, undefined);
    } else {
      setHandlers(eventName, list);
    }
  };
  const hot = hotReload(hooks.hotData);
  const ctx = {
    on,
    off,
    hot: hot.api,
  };

  // `console`: each call joins its arguments into one message for the host sink.
  const writeConsole = hooks.console;
  const describe = (value) => {
    if (typeof value === "string") {
      return value;
    }
    if (value instanceof Error) {
      const stack = typeof value.stack === "string" ? value.stack.trimEnd() : "";
      return stack === "" ? `${value.name}: ${value.message}` : `${value.name}: ${value.message}\n${stack}`;
    }
    try {
      const json = JSON.stringify(value);
      if (json !== undefined) {
        return json;
      }
    } catch {
      // Cycles and BigInt have no JSON form.
    }
    return String(value);
  };
  const consoleMethod = (level) => (...args) => {
    writeConsole(level, args.map(describe).join(" "));
  };
  const console = {
    debug: consoleMethod("debug"),
    log: consoleMethod("log"),
    info: consoleMethod("info"),
    warn: consoleMethod("warn"),
    error: consoleMethod("error"),
  };

  // Timers on the engine clock, which only the host's `advance_timers` moves. The
  // engine enters this context only once `schedule` says a timer is due.
  //
  // The live timers sit in an array, in creation order, and in a Map by id for
  // `clearTimeout`; a cleared or finished timer is only marked and swept after the next
  // run. Plain arrays and indexed loops allocate nothing: iterating a Map creates an
  // iterator and a result object per step, and the engine runs this every time a timer
  // is due.
  const now = hooks.now;
  const schedule = hooks.schedule;
  let live = [];
  const byId = new Map();
  let nextTimerId = 1;
  let earliest = Infinity;
  let finished = 0;
  const reschedule = () => {
    let due = Infinity;
    for (let i = 0; i < live.length; i++) {
      const timer = live[i];
      if (!timer.cancelled && timer.due < due) {
        due = timer.due;
      }
    }
    earliest = due;
    schedule(due);
  };
  const addTimer = (name, callback, delay, args, repeats) => {
    if (typeof callback !== "function") {
      throw new TypeError(`${name} expects a function callback`);
    }
    const wait = Math.max(0, Number(delay) || 0);
    const id = nextTimerId++;
    const timer = {
      id,
      callback,
      args,
      due: now() + wait,
      interval: repeats ? wait : undefined,
      cancelled: false,
    };
    live.push(timer);
    byId.set(id, timer);
    if (timer.due < earliest) {
      earliest = timer.due;
      schedule(earliest);
    }
    return id;
  };
  const retire = (timer) => {
    timer.cancelled = true;
    byId.delete(timer.id);
    finished++;
  };
  const clearTimer = (id) => {
    const timer = byId.get(id);
    if (timer !== undefined) {
      retire(timer);
      if (timer.due === earliest) {
        reschedule();
      }
    }
  };
  const setTimeout = (callback, delay, ...args) =>
    addTimer("setTimeout", callback, delay, args, false);
  const setInterval = (callback, delay, ...args) =>
    addTimer("setInterval", callback, delay, args, true);
  let failed = false;
  let failure;
  const fire = (timer) => {
    if (timer.cancelled) {
      return; // cleared by a callback that ran before it
    }
    if (timer.interval === undefined) {
      retire(timer);
    } else {
      timer.due += timer.interval;
    }
    try {
      if (timer.args.length === 0) {
        timer.callback();
      } else {
        timer.callback(...timer.args);
      }
    } catch (error) {
      if (!failed) {
        failed = true;
        failure = error;
      }
    }
  };
  // Fires the timers due at `time` in due order, then creation order; every one
  // runs even after one throws, and the first error is rethrown. A timer a callback
  // adds waits for the next call.
  const runTimers = (time) => {
    let only;
    let due;
    for (let i = 0; i < live.length; i++) {
      const timer = live[i];
      if (timer.cancelled || timer.due > time) {
        continue;
      }
      if (only === undefined) {
        only = timer;
      } else if (due === undefined) {
        due = [only, timer];
      } else {
        due.push(timer);
      }
    }
    if (only === undefined) {
      reschedule();
      return;
    }
    failed = false;
    failure = undefined;
    if (due === undefined) {
      fire(only);
    } else {
      due.sort((left, right) => left.due - right.due || left.id - right.id);
      for (let i = 0; i < due.length; i++) {
        fire(due[i]);
      }
    }
    if (finished > 0) {
      let kept = 0;
      for (let i = 0; i < live.length; i++) {
        if (!live[i].cancelled) {
          live[kept++] = live[i];
        }
      }
      live.length = kept;
      finished = 0;
    }
    reschedule();
    if (failed) {
      const error = failure;
      failed = false;
      failure = undefined;
      throw error;
    }
  };

  // The host functions bound to this script: what the host module imports of a group
  // script read, and what `installGlobals` exposes to a script in a context of its own.
  const native = hooks.native;

  return {
    ctx,
    console,
    setTimeout,
    setInterval,
    clearTimeout: clearTimer,
    clearInterval: clearTimer,
    native,
    on,
    hotSave: hot.hooks.save,
    hotDispose: hot.hooks.dispose,
    timersRun: runTimers,
  };
}

// Exposes a script's environment as globals, for a script in a context of its own.
// Host functions are reachable three ways that all end in the same native function:
// host module imports, namespaced globals (`user.find(...)`) and the `__host` bridge used
// by the generated SDK.
export function installGlobals(env) {
  globalThis.ctx = env.ctx;
  globalThis.console = env.console;
  globalThis.setTimeout = env.setTimeout;
  globalThis.setInterval = env.setInterval;
  globalThis.clearTimeout = env.clearTimeout;
  globalThis.clearInterval = env.clearInterval;
  globalThis.__manivelle_on = env.on;
  globalThis.__manivelle_native = env.native;
  const native = env.native;

  const hostFunction = (name) => {
    const fn = native[name];
    if (typeof fn !== "function") {
      throw new Error(`missing host function: ${name}`);
    }
    return fn;
  };

  globalThis.__host = {
    callValue: (name, input) => hostFunction(name)(input === undefined ? null : input),
  };

  const namespaceChild = (parent, segment) => {
    const existing = parent[segment];
    if (existing !== null && (typeof existing === "object" || typeof existing === "function")) {
      return existing;
    }
    return (parent[segment] = {});
  };

  for (const name of Object.keys(native)) {
    const segments = name.split(".");
    const leaf = segments.pop();
    let parent = globalThis;
    for (const segment of segments) {
      parent = namespaceChild(parent, segment);
    }
    parent[leaf] = native[name];
  }
}
