//! Codec opt-ins (`json`, custom `with`) and the error messages of decoding.

use manivelle::js::Value as JsValue;
use manivelle::{JsDecode, TsSchema, TsType};
use serde::{Deserialize, Serialize};

use super::support::codec::{assert_contract, assert_decodes_like_serde, decode_error, with_js};
use super::types::{Account, Email, ExternalId, Item, Log, Player, Position, Wrapper};

#[test]
fn json_and_custom_codec_opt_ins_follow_serde() {
    assert_contract(&[Account {
        external: ExternalId(42),
        mask: 0xff00,
        call_sign: String::from("falcon"),
        balance: -12,
    }]);
    assert_decodes_like_serde::<Account>(
        r#"{"external":"nope","mask":"ff","call_sign":"X","balance":1}"#,
    );
    assert_decodes_like_serde::<Account>(r#"{"mask":"ff","call_sign":"X","balance":1}"#);

    assert_contract(&[Email(String::from("a@b.c"))]);
    assert_decodes_like_serde::<Email>(r#""not-an-email""#);
    assert_eq!(Email::ts_type(), TsType::String);
}

#[test]
fn decode_errors_name_the_field_path() {
    let missing =
        decode_error::<Player>(r#"{ playerId: 1, displayName: "a", position: { x: 1 } }"#);
    assert!(missing.contains("position.y: missing field"), "{missing}");

    let range =
        decode_error::<Player>(r#"{ playerId: 1, displayName: "a", position: { x: 300, y: 0 } }"#);
    assert!(range.contains("position.x: "), "{range}");

    let items = decode_error::<Wrapper<Vec<Option<Item>>>>(
        r#"{ inner: [null, { id: "7", label: "x" }], count: 1 }"#,
    );
    assert!(items.contains("inner[1].id: "), "{items}");

    let tag = decode_error::<Log>(r#"{ event: { kind: "Exploded" }, lookup: 1, command: "stop" }"#);
    assert!(
        tag.contains("event.kind: unknown variant `Exploded`"),
        "{tag}"
    );

    let untagged =
        decode_error::<Log>(r#"{ event: { kind: "Ping" }, lookup: true, command: "stop" }"#);
    assert!(
        untagged.contains("lookup: data did not match any variant of untagged enum Lookup"),
        "{untagged}"
    );

    let payload = decode_error::<Log>(
        r#"{ event: { kind: "Ping" }, lookup: 1, command: { move: { x: 1 } } }"#,
    );
    assert!(
        payload.contains("command.move.y: missing field"),
        "{payload}"
    );
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
struct Node {
    next: Option<Box<Node>>,
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
enum Chain {
    End,
    Link(Box<Chain>),
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
struct Tree {
    children: Vec<Tree>,
}

/// JavaScript that builds `depth` levels around `innermost`, each written `wrap(inner)`.
fn nested(depth: usize, innermost: &str, wrap: &str) -> String {
    format!(
        "(() => {{ let x = {innermost}; for (let i = 0; i < {depth}; i++) x = {wrap}; return x; }})()"
    )
}

#[test]
fn a_recursive_type_refuses_values_nested_beyond_the_decoding_depth() {
    // 200 levels are past the limit and still shallow enough to decode safely without
    // one; 100 000 would overflow the stack.
    for depth in [200, 100_000] {
        let node = decode_error::<Node>(&nested(depth, "null", "{ next: x }"));
        let chain = decode_error::<Chain>(&nested(depth, r#""End""#, "({ Link: x })"));
        let tree = decode_error::<Tree>(&nested(depth, "{ children: [] }", "{ children: [x] }"));

        for error in [node, chain, tree] {
            assert!(
                error.contains("nested too deeply"),
                "depth {depth}: {error}"
            );
        }
    }
}

#[test]
fn a_refused_deep_value_does_not_shrink_the_depth_left_to_the_next_decode() {
    decode_error::<Node>(&nested(200, "null", "{ next: x }"));

    // Fifty levels is within the limit, and decodes as serde_json does.
    let text = format!("{}null{}", r#"{"next":"#.repeat(50), "}".repeat(50));
    assert_decodes_like_serde::<Node>(&text);
}

#[test]
fn a_field_is_read_from_the_object_itself_and_never_from_its_prototype() {
    let missing = decode_error::<Position>("Object.create({ x: 1, y: 2 })");
    assert!(missing.contains("x: missing field"), "{missing}");

    with_js(|ctx| {
        ctx.eval::<(), _>(
            "globalThis.hits = 0;
             Object.defineProperty(Object.prototype, 'y', {
                 get() { hits += 1; return 5; },
                 configurable: true,
             });",
        )
        .expect("install an inherited getter named like a field");
        let own = ctx.eval::<JsValue<'_>, _>("({ x: 1, y: 2 })").expect("own");
        let absent = ctx.eval::<JsValue<'_>, _>("({ x: 1 })").expect("absent");

        let decoded = Position::decode_js(ctx, own).expect("a field of the object decodes");
        let error = Position::decode_js(ctx, absent).expect_err("an inherited field is absent");

        assert_eq!(decoded, Position { x: 1, y: 2 });
        assert!(error.to_string().contains("y: missing field"), "{error}");
        let hits: u32 = ctx.globals().get("hits").expect("read the counter");
        assert_eq!(hits, 0, "the inherited getter ran during decoding");
    });
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
enum ProtoVariant {
    #[serde(rename = "__proto__")]
    Variant(u8),
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "__proto__")]
enum ProtoTag {
    A { n: u8 },
}

#[derive(Debug, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind", content = "__proto__")]
enum ProtoContent {
    A(Vec<u8>),
}

#[test]
fn enum_names_called_proto_stay_own_properties_like_in_serde() {
    assert_contract(&[ProtoVariant::Variant(7)]);
    assert_contract(&[ProtoTag::A { n: 7 }]);
    assert_contract(&[ProtoContent::A(vec![1, 2])]);
}

/// Decodes through serde as `u8`, and natively fails at another path than serde does.
#[derive(Debug, PartialEq)]
struct Skewed(u8);

impl<'de> Deserialize<'de> for Skewed {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        u8::deserialize(deserializer).map(Skewed)
    }
}

impl JsDecode for Skewed {
    fn decode_js<'js>(
        _ctx: &manivelle::js::Ctx<'js>,
        _value: JsValue<'js>,
    ) -> Result<Self, manivelle::CodecError> {
        Err(manivelle::CodecError::from_js("string", "u8", "expected integer").at("elsewhere"))
    }
}

#[test]
#[should_panic(expected = "fails elsewhere")]
fn the_serde_differential_refuses_a_failure_at_another_path() {
    assert_decodes_like_serde::<Skewed>(r#""text""#);
}
