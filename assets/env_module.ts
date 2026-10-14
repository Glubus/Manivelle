/**
 * What a script imports to reach its own `ctx`, `console` and timers. A script in a
 * context group has no such globals, since its context is shared; a script in a context
 * of its own can use either.
 */
declare module "manivelle:env" {
  export const ctx: __CTX__;
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
