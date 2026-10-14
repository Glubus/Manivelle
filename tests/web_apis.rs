//! The web APIs a script context gets with `ScriptBuiltins::web`: `structuredClone`,
//! `TextEncoder` and `TextDecoder`.

use manivelle::{Engine, EngineError, EngineOptions, ScriptBuiltins};
use serde_json::{Value, json};

/// Loads `body` as the module `probe` and calls its exported `run()`.
fn run(body: &str) -> Result<Value, EngineError> {
    run_with(ScriptBuiltins::ALL, body)
}

fn run_with(builtins: ScriptBuiltins, body: &str) -> Result<Value, EngineError> {
    let mut engine = Engine::new(&EngineOptions {
        builtins,
        ..EngineOptions::default()
    })
    .expect("create engine");
    engine.load_script(
        "probe",
        &format!("export function run(): unknown {{ {body} }}"),
    )?;
    engine.call("probe", "run", ())
}

#[test]
fn the_text_encoder_writes_utf_8() {
    let bytes = run(r#"return Array.from(new TextEncoder().encode("hé €😀"));"#).expect("run");

    assert_eq!(
        bytes,
        json!([104, 195, 169, 32, 226, 130, 172, 240, 159, 152, 128])
    );
}

#[test]
fn the_text_encoder_replaces_lone_surrogates_with_the_replacement_character() {
    let result = run(r#"
        const lone = "a\ud800b";
        const into = new Uint8Array(8);
        const { written } = new TextEncoder().encodeInto(lone, into);
        return [Array.from(new TextEncoder().encode(lone)), Array.from(into.subarray(0, written))];
    "#)
    .expect("run");

    let expected = json!([97, 239, 191, 189, 98]);
    assert_eq!(result, json!([expected, expected]));
}

#[test]
fn encode_into_stops_before_a_code_point_that_does_not_fit() {
    let result = run(r#"
        const encoder = new TextEncoder();
        const small = new Uint8Array(3);
        const exact = new Uint8Array(4);
        const emoji = new Uint8Array(4);
        return [
            encoder.encodeInto("a€b", small),
            encoder.encodeInto("a€b", exact),
            encoder.encodeInto("😀", emoji),
            Array.from(small),
        ];
    "#)
    .expect("run");

    assert_eq!(
        result,
        json!([
            { "read": 1, "written": 1 },
            { "read": 2, "written": 4 },
            { "read": 2, "written": 4 },
            [97, 0, 0],
        ])
    );
}

#[test]
fn the_text_decoder_reads_utf_8_from_buffers_and_views() {
    let result = run(r#"
        const bytes = new TextEncoder().encode("grüße €");
        const decoder = new TextDecoder();
        const framed = new Uint8Array([0, 0, ...bytes, 0]);
        return [
            decoder.decode(bytes),
            decoder.decode(bytes.buffer),
            decoder.decode(framed.subarray(2, 2 + bytes.length)),
            decoder.decode(),
            new TextDecoder("UTF8").encoding,
        ];
    "#)
    .expect("run");

    assert_eq!(
        result,
        json!(["grüße €", "grüße €", "grüße €", "", "utf-8"])
    );
}

#[test]
fn the_text_decoder_handles_the_bom_and_invalid_input_as_the_web_does() {
    let result = run(r#"
        const bom = new Uint8Array([0xef, 0xbb, 0xbf, 0x61]);
        const invalid = new Uint8Array([0x61, 0xff, 0x62]);
        let fatal = "none";
        try {
            new TextDecoder("utf-8", { fatal: true }).decode(invalid);
        } catch (error) {
            fatal = (error as Error).name;
        }
        return [
            new TextDecoder().decode(bom),
            new TextDecoder("utf-8", { ignoreBOM: true }).decode(bom).length,
            new TextDecoder().decode(invalid),
            fatal,
        ];
    "#)
    .expect("run");

    assert_eq!(result, json!(["a", 2, "a\u{fffd}b", "TypeError"]));
}

#[test]
fn the_text_decoder_rejects_what_it_does_not_support() {
    let result = run(r#"
        const failure = (action: () => void): string => {
            try {
                action();
                return "none";
            } catch (error) {
                return (error as Error).name;
            }
        };
        return [
            failure(() => new TextDecoder("latin1")),
            failure(() => new TextDecoder().decode(new Uint8Array(1), { stream: true })),
            failure(() => new TextDecoder().decode("text" as any)),
            failure(() => new TextEncoder().encodeInto("a", [] as any)),
        ];
    "#)
    .expect("run");

    assert_eq!(
        result,
        json!(["RangeError", "TypeError", "TypeError", "TypeError"])
    );
}

#[test]
fn structured_clone_copies_deeply() {
    let result = run(r#"
        const original = { list: [1, { deep: true }], map: new Map([["k", { v: 1 }]]), set: new Set([1, 2]) };
        const copy = structuredClone(original);
        (original.list[1] as any).deep = false;
        original.map.get("k")!.v = 2;
        original.set.add(3);
        return [
            copy !== original,
            copy.list[1],
            copy.map.get("k"),
            Array.from(copy.set),
        ];
    "#)
    .expect("run");

    assert_eq!(result, json!([true, { "deep": true }, { "v": 1 }, [1, 2]]));
}

#[test]
fn structured_clone_keeps_cycles_and_shared_references() {
    let result = run(r#"
        const shared = { n: 1 };
        const root: any = { a: shared, b: shared };
        root.self = root;
        const copy = structuredClone(root);
        return [copy.self === copy, copy.a === copy.b, copy.a !== shared];
    "#)
    .expect("run");

    assert_eq!(result, json!([true, true, true]));
}

#[test]
fn structured_clone_copies_dates_regexps_buffers_and_errors() {
    let result = run(r#"
        const buffer = new Uint8Array([1, 2, 3, 4]);
        const view = buffer.subarray(1, 3);
        const original = { when: new Date(86400000), pattern: /a+/gi, buffer, view, error: new RangeError("bad", { cause: "why" }) };
        const copy = structuredClone(original);
        buffer[1] = 99;
        return [
            copy.when instanceof Date && copy.when.getTime(),
            copy.pattern.source + "/" + copy.pattern.flags,
            Array.from(copy.buffer),
            Array.from(copy.view),
            copy.view.buffer === copy.buffer.buffer,
            copy.error instanceof RangeError,
            copy.error.message,
            copy.error.cause,
        ];
    "#)
    .expect("run");

    assert_eq!(
        result,
        json!([
            86400000,
            "a+/gi",
            [1, 2, 3, 4],
            [2, 3],
            true,
            true,
            "bad",
            "why"
        ])
    );
}

#[test]
fn structured_clone_turns_class_instances_into_plain_objects_and_keeps_special_keys() {
    let result = run(r#"
        class Point { constructor(public x: number, public y: number) {} get sum() { return this.x + this.y; } }
        const copy = structuredClone({ point: new Point(1, 2), undef: undefined, ["__proto__x"]: 1 }) as any;
        const own = JSON.parse('{"__proto__": {"polluted": true}}');
        const proto = structuredClone(own) as any;
        return [
            Object.getPrototypeOf(copy.point) === Object.prototype,
            copy.point.sum,
            "undef" in copy && copy.undef === undefined,
            Object.prototype.hasOwnProperty.call(proto, "__proto__"),
            ({} as any).polluted,
        ];
    "#)
    .expect("run");

    assert_eq!(result, json!([true, null, true, true, null]));
}

#[test]
fn structured_clone_keeps_holes_and_rejects_what_cannot_be_cloned() {
    let result = run(r#"
        const failure = (value: unknown, options?: { transfer: ArrayBuffer[] }): string => {
            try {
                structuredClone(value, options);
                return "none";
            } catch (error) {
                return (error as Error).name;
            }
        };
        const sparse = structuredClone([1, , 3]);
        return [
            0 in sparse && 1 in sparse,
            sparse.length,
            failure(() => {}),
            failure(Symbol("s")),
            failure({ nested: new Promise(() => {}) }),
            failure(new WeakMap()),
            failure(1n) === "none",
            failure(1, { transfer: [new ArrayBuffer(1)] }),
        ];
    "#)
    .expect("run");

    assert_eq!(
        result,
        json!([
            false,
            3,
            "DataCloneError",
            "DataCloneError",
            "DataCloneError",
            "DataCloneError",
            true,
            "DataCloneError"
        ])
    );
}

#[test]
fn a_web_global_a_script_assigns_before_reading_any_stays_its_own() {
    let result = run(r#"
        const scope = globalThis as any;
        scope.structuredClone = () => "mine";
        const encoder = new TextEncoder();
        const descriptor = Object.getOwnPropertyDescriptor(globalThis, "TextDecoder")!;
        return [
            structuredClone(1),
            Array.from(encoder.encode("a")),
            "value" in descriptor && descriptor.writable && !descriptor.enumerable,
            Object.keys(globalThis).includes("TextEncoder"),
            new TextEncoder().constructor === TextEncoder,
        ];
    "#)
    .expect("run");

    assert_eq!(result, json!(["mine", [97], true, false, true]));
}

#[test]
fn the_web_apis_follow_the_web_built_in_switch() {
    let probe = r#"return [typeof structuredClone, typeof TextEncoder, typeof TextDecoder];"#;
    let without_web = ScriptBuiltins {
        web: false,
        ..ScriptBuiltins::ALL
    };
    let without_typed_arrays = ScriptBuiltins {
        typed_arrays: false,
        ..ScriptBuiltins::ALL
    };

    assert_eq!(
        run_with(ScriptBuiltins::ALL, probe).expect("all"),
        json!(["function", "function", "function"])
    );
    assert_eq!(
        run_with(without_web, probe).expect("no web"),
        json!(["undefined", "undefined", "undefined"])
    );
    assert_eq!(
        run_with(without_typed_arrays, probe).expect("no typed arrays"),
        json!(["function", "undefined", "undefined"])
    );
}

#[test]
fn scripts_of_a_group_share_the_apis_without_installing_them_again() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    for id in ["one", "two"] {
        engine
            .load_script_in(
                "pack",
                id,
                "export const same = (): boolean => typeof TextEncoder === 'function' && typeof structuredClone === 'function';",
            )
            .expect("load");
    }

    for id in ["one", "two"] {
        assert!(engine.call::<bool>(id, "same", ()).expect("call"));
    }
}
