type Frame = { dt: number; tick: number; };

declare namespace game {
  /** The best score on record, answered by another thread. */
  export function highScore(input: void): Promise<number>;
}

type Status = null;

declare module "game" {
  export const game: {
    /** The best score on record, answered by another thread. */
    highScore(input: void): Promise<number>;
  };
}

type HostEvents = {
  /**
   * The per-frame event payload, encoded natively into a JavaScript object. `#[host_event]`
   * names the event, so `emit` takes `Frame::NAME` instead of a string that could be misspelled,
   * and the `frame` handler of `hud.ts` gets its parameter typed from this struct.
   */
  "frame": Frame;
  /** The `status` request: every script replies with a line of text. */
  "status": Status;
};

type HostReplies = {
  "status": string;
};

/**
 * What a handler of event `K` returns: the reply type of a request registered with
 * `request`, nothing for a plain callback. An `async` handler returns a
 * Promise of it.
 */
type HostEventReply<K extends keyof HostEvents> = K extends keyof HostReplies
  ? HostReplies[K] | Promise<HostReplies[K]>
  : void | Promise<void>;

type HostEventHandler<K extends keyof HostEvents> = (
  payload: HostEvents[K],
) => HostEventReply<K>;

type HostEventContext = {
  /** Registers `handler` for `event`; handlers run in registration order. */
  on<K extends keyof HostEvents>(event: K, handler: HostEventHandler<K>): void;
  /**
   * Removes the first registration of `handler` for `event`; does nothing when it is
   * not registered. A delivery already in progress still runs it.
   */
  off<K extends keyof HostEvents>(event: K, handler: HostEventHandler<K>): void;
};

/** State a script keeps across hot reloads, through `ctx.hot`. */
type HostHotContext = {
  /**
   * What the previous version's `save` callback returned, set before this version's
   * code runs; `undefined` on a first load. It is the same object, not a copy.
   */
  readonly data: unknown;
  /**
   * Registers the callback that runs on this version when a reload replaces it; its
   * result becomes the new version's `ctx.hot.data`. It runs before the new version
   * is known to load, so it must not have side effects. The last registration wins.
   */
  save(snapshot: () => unknown): void;
  /**
   * Registers a cleanup that runs on this version once a new version has loaded, or
   * when the script is unloaded. Cleanups run in registration order.
   */
  dispose(cleanup: () => void): void;
};

declare const ctx: HostEventContext & { readonly hot: HostHotContext };

/**
 * What a script imports to reach its own `ctx`, `console` and timers. A script in a
 * context group has no such globals, since its context is shared; a script in a context
 * of its own can use either.
 */
declare module "manivelle:env" {
  export const ctx: HostEventContext & { readonly hot: HostHotContext };
  export const console: {
    debug(...data: unknown[]): void;
    log(...data: unknown[]): void;
    info(...data: unknown[]): void;
    warn(...data: unknown[]): void;
    error(...data: unknown[]): void;
  };
  export function setTimeout(callback: (...args: any[]) => void, delay?: number, ...args: any[]): number;
  export function setInterval(callback: (...args: any[]) => void, delay?: number, ...args: any[]): number;
  export function clearTimeout(id: number | undefined): void;
  export function clearInterval(id: number | undefined): void;
}
