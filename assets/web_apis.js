// The web platform APIs a plain QuickJS context lacks and mod code and npm packages
// expect: `structuredClone`, `TextEncoder` and `TextDecoder`. A module of its own, compiled
// once per engine and loaded into a context only when a script first reads one of the
// globals (see `installWebApis` in the context prelude), so a context that never uses
// them pays nothing for their code. `natives` is the pair of UTF-8 conversions the engine
// provides. Built-ins are captured here, so a script that patches `Map.prototype` later
// cannot change what these do.

// Returns the implementations, by global name.
export function create(natives) {
  const apis = { structuredClone: makeStructuredClone() };
  if (typeof Uint8Array === "function") {
    const [TextEncoder, TextDecoder] = makeTextCoders(natives);
    apis.TextEncoder = TextEncoder;
    apis.TextDecoder = TextDecoder;
  }
  return apis;
}

function dataCloneError(message) {
  if (typeof DOMException === "function") {
    return new DOMException(message, "DataCloneError");
  }
  const error = new Error(message);
  error.name = "DataCloneError";
  return error;
}

// `structuredClone(value)`: a deep copy that keeps cycles and shared references, and
// copies the built-ins the web algorithm copies (primitives and their wrappers, `Date`,
// `RegExp`, `Map`, `Set`, `Array`, `ArrayBuffer`, typed arrays and `DataView`, errors,
// plain objects). A class instance becomes a plain object. A function, a symbol, a
// `Promise` or a weak collection throws a `DataCloneError`. Transferring is not supported.
function makeStructuredClone() {
  const toStringTag = Object.prototype.toString;
  const isArray = Array.isArray;
  const keys = Object.keys;
  const defineProperty = Object.defineProperty;
  const has = Function.prototype.call.bind(Object.prototype.hasOwnProperty);
  const hasMap = typeof Map === "function";
  const hasSet = typeof Set === "function";
  const hasDate = typeof Date === "function";
  const hasRegExp = typeof RegExp === "function";
  const hasBuffers = typeof ArrayBuffer === "function";
  const mapForEach = hasMap && Map.prototype.forEach;
  const mapSize = hasMap && Object.getOwnPropertyDescriptor(Map.prototype, "size").get;
  const mapSet = hasMap && Map.prototype.set;
  const setForEach = hasSet && Set.prototype.forEach;
  const setSize = hasSet && Object.getOwnPropertyDescriptor(Set.prototype, "size").get;
  const setAdd = hasSet && Set.prototype.add;
  const dateTime = hasDate && Date.prototype.getTime;
  const regExpSource = hasRegExp && Object.getOwnPropertyDescriptor(RegExp.prototype, "source").get;
  const regExpFlags = hasRegExp && Object.getOwnPropertyDescriptor(RegExp.prototype, "flags").get;
  const bufferLength = hasBuffers && Object.getOwnPropertyDescriptor(ArrayBuffer.prototype, "byteLength").get;
  const bufferSlice = hasBuffers && ArrayBuffer.prototype.slice;
  const isView = hasBuffers && ArrayBuffer.isView;
  const viewTags = new Set([
    "Int8Array", "Uint8Array", "Uint8ClampedArray", "Int16Array", "Uint16Array",
    "Int32Array", "Uint32Array", "Float32Array", "Float64Array", "BigInt64Array",
    "BigUint64Array", "DataView",
  ]);
  const viewConstructors = new Map();
  for (const name of viewTags) {
    if (typeof globalThis[name] === "function") {
      viewConstructors.set(name, globalThis[name]);
    }
  }
  const errorConstructors = new Map([
    ["Error", Error], ["EvalError", EvalError], ["RangeError", RangeError],
    ["ReferenceError", ReferenceError], ["SyntaxError", SyntaxError],
    ["TypeError", TypeError], ["URIError", URIError],
  ]);
  const uncloneable = new Set([
    "Promise", "WeakMap", "WeakSet", "WeakRef", "FinalizationRegistry", "Symbol",
    "Generator", "AsyncGenerator", "Proxy",
  ]);
  const attempt = (read) => {
    try {
      return { value: read() };
    } catch {
      return undefined;
    }
  };

  const describe = (value) => {
    try {
      return String(value);
    } catch {
      return "#<Object>";
    }
  };

  return function structuredClone(value, options) {
    if (arguments.length === 0) {
      throw new TypeError("structuredClone requires at least 1 argument");
    }
    if (options !== undefined && options !== null && options.transfer !== undefined) {
      if (options.transfer.length) {
        throw dataCloneError("structuredClone does not support transferring");
      }
    }
    const seen = new Map();
    const remember = (input, output) => {
      seen.set(input, output);
      return output;
    };
    const copyProperties = (input, output) => {
      for (const key of keys(input)) {
        defineProperty(output, key, {
          value: clone(input[key]),
          writable: true,
          enumerable: true,
          configurable: true,
        });
      }
      return output;
    };
    const clone = (input) => {
      const type = typeof input;
      if (type === "function" || type === "symbol") {
        throw dataCloneError(`${describe(input)} could not be cloned.`);
      }
      if (type !== "object" || input === null) {
        return input;
      }
      if (seen.has(input)) {
        return seen.get(input);
      }
      const tag = toStringTag.call(input).slice(8, -1);
      if (uncloneable.has(tag)) {
        throw dataCloneError(`#<${tag}> could not be cloned.`);
      }
      if (isArray(input)) {
        return copyProperties(input, remember(input, new Array(input.length)));
      }
      if (hasDate && tag === "Date") {
        const time = attempt(() => dateTime.call(input));
        if (time) {
          return remember(input, new Date(time.value));
        }
      }
      if (hasRegExp && tag === "RegExp") {
        const source = attempt(() => regExpSource.call(input));
        if (source) {
          return remember(input, new RegExp(source.value, regExpFlags.call(input)));
        }
      }
      if (hasMap && tag === "Map" && attempt(() => mapSize.call(input))) {
        const output = remember(input, new Map());
        const entries = [];
        mapForEach.call(input, (item, key) => entries.push([key, item]));
        for (const [key, item] of entries) {
          mapSet.call(output, clone(key), clone(item));
        }
        return output;
      }
      if (hasSet && tag === "Set" && attempt(() => setSize.call(input))) {
        const output = remember(input, new Set());
        const items = [];
        setForEach.call(input, (item) => items.push(item));
        for (const item of items) {
          setAdd.call(output, clone(item));
        }
        return output;
      }
      if (hasBuffers && tag === "ArrayBuffer" && attempt(() => bufferLength.call(input))) {
        return remember(input, bufferSlice.call(input, 0));
      }
      if (isView && isView(input) && viewConstructors.has(tag)) {
        const buffer = clone(input.buffer);
        const View = viewConstructors.get(tag);
        const output =
          tag === "DataView"
            ? new View(buffer, input.byteOffset, input.byteLength)
            : new View(buffer, input.byteOffset, input.length);
        return remember(input, output);
      }
      if (tag === "Error") {
        const Constructor = errorConstructors.get(input.name) || Error;
        const output = remember(input, new Constructor(has(input, "message") ? String(input.message) : undefined));
        if (typeof input.stack === "string") {
          defineProperty(output, "stack", {
            value: input.stack,
            writable: true,
            configurable: true,
          });
        }
        if (has(input, "cause")) {
          defineProperty(output, "cause", {
            value: clone(input.cause),
            writable: true,
            configurable: true,
          });
        }
        return output;
      }
      if (tag === "Boolean" || tag === "Number" || tag === "String" || tag === "BigInt") {
        const primitive = attempt(() => input.valueOf());
        if (primitive && typeof primitive.value !== "object") {
          return remember(input, Object(primitive.value));
        }
      }
      return copyProperties(input, remember(input, {}));
    };
    return clone(value);
  };
}

// The string with each lone surrogate replaced by U+FFFD: what `String.prototype.toWellFormed`
// does, for a context without it.
function replaceLoneSurrogates(text) {
  let output = "";
  for (let index = 0; index < text.length; index++) {
    const unit = text.charCodeAt(index);
    if (unit >= 0xd800 && unit <= 0xdbff) {
      const next = text.charCodeAt(index + 1);
      if (next >= 0xdc00 && next <= 0xdfff) {
        output += text[index] + text[index + 1];
        index++;
        continue;
      }
    }
    output += unit >= 0xd800 && unit <= 0xdfff ? "\ufffd" : text[index];
  }
  return output;
}

// `TextEncoder` and `TextDecoder`, UTF-8 only and without streaming, over the engine's
// native conversions.
function makeTextCoders(natives) {
  const { decode } = natives;
  // A string with a lone surrogate is not valid Unicode: the web replaces each with
  // U+FFFD, where the native conversion would fail.
  const encode = (text) => {
    const wellFormed =
      typeof text.toWellFormed === "function" ? text.toWellFormed() : replaceLoneSurrogates(text);
    return natives.encode(wellFormed);
  };
  const utf8Labels = new Set([
    "unicode-1-1-utf-8", "unicode11utf8", "unicode20utf8", "utf-8", "utf8", "x-unicode20utf8",
  ]);

  class TextEncoder {
    get encoding() {
      return "utf-8";
    }

    encode(input = "") {
      return encode(String(input));
    }

    encodeInto(source, destination) {
      if (!(destination instanceof Uint8Array)) {
        throw new TypeError("The destination must be a Uint8Array");
      }
      const bytes = encode(String(source));
      let written = Math.min(bytes.length, destination.length);
      // Never write half a code point.
      while (written > 0 && written < bytes.length && (bytes[written] & 0xc0) === 0x80) {
        written--;
      }
      destination.set(bytes.subarray(0, written));
      let read = 0;
      for (let index = 0; index < written; ) {
        const lead = bytes[index];
        index += lead < 0x80 ? 1 : lead < 0xe0 ? 2 : lead < 0xf0 ? 3 : 4;
        read += lead >= 0xf0 ? 2 : 1;
      }
      return { read, written };
    }
  }

  class TextDecoder {
    #fatal;
    #ignoreBOM;

    constructor(label = "utf-8", options = {}) {
      if (!utf8Labels.has(String(label).trim().toLowerCase())) {
        throw new RangeError(`The encoding label provided ('${label}') is invalid.`);
      }
      this.#fatal = Boolean(options && options.fatal);
      this.#ignoreBOM = Boolean(options && options.ignoreBOM);
    }

    get encoding() {
      return "utf-8";
    }

    get fatal() {
      return this.#fatal;
    }

    get ignoreBOM() {
      return this.#ignoreBOM;
    }

    decode(input = new Uint8Array(0), options) {
      if (options && options.stream) {
        throw new TypeError("TextDecoder does not support streaming");
      }
      let bytes;
      if (input instanceof ArrayBuffer) {
        bytes = new Uint8Array(input);
      } else if (ArrayBuffer.isView(input)) {
        bytes = new Uint8Array(input.buffer, input.byteOffset, input.byteLength);
      } else {
        throw new TypeError("The provided value is not of type '(ArrayBuffer or ArrayBufferView)'");
      }
      return decode(bytes, this.#fatal, this.#ignoreBOM);
    }
  }

  for (const [constructor, name] of [[TextEncoder, "TextEncoder"], [TextDecoder, "TextDecoder"]]) {
    Object.defineProperty(constructor.prototype, Symbol.toStringTag, {
      value: name,
      configurable: true,
    });
  }
  return [TextEncoder, TextDecoder];
}
