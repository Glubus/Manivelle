// Installed on globalThis by the Manivelle engine before any script runs.
declare const __host: {
  callValue(name: string, input: unknown): unknown;
};

function __hostCall<T>(name: string, input?: unknown): T {
  return __host.callValue(name, input === undefined ? null : input) as T;
}
