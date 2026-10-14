//! `#[derive(TsSchema)]` codecs follow the `serde_json` reference: encoding equals
//! `serde_json::to_string` then `JSON.parse`, decoding equals `JSON.stringify` then
//! `serde_json::from_str`, and the derived schema accepts what serde writes.

#![cfg(feature = "derive")]

use std::collections::BTreeMap;
use std::fmt::Debug;
use std::marker::PhantomData;

use manivelle::js::{Context, Ctx, Runtime, Value as JsValue};
use manivelle::{JsDecode, JsEncode, TsSchema, TsType};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value, json};

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn with_js<R>(run: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let runtime = Runtime::new().expect("create runtime");
    let context = Context::full(&runtime).expect("create context");
    context.with(|ctx| run(&ctx))
}

/// Numbers compare by value: `3` and `3.0` are the same JSON number.
fn normalize(value: Value) -> Value {
    match value {
        Value::Number(number) => Number::from_f64(number.as_f64().expect("finite number"))
            .map_or(Value::Number(number), Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, normalize(value)))
                .collect(),
        ),
        other => other,
    }
}

fn assert_encodes_like_serde<T: JsEncode + Serialize + Debug>(value: &T) {
    let reference = serde_json::to_value(value).expect("serialize reference");
    let native = with_js(|ctx| {
        let encoded = value.encode_js(ctx).expect("encode natively");
        let text = ctx
            .json_stringify(encoded)
            .expect("stringify")
            .expect("JSON text")
            .to_string()
            .expect("utf-8 JSON text");
        serde_json::from_str::<Value>(&text).expect("parse native JSON")
    });
    assert_eq!(
        normalize(native),
        normalize(reference),
        "encoding {value:?}"
    );
}

/// Decoding `JSON.parse(text)` natively matches `serde_json::from_str(text)`, including
/// whether it fails.
fn assert_decodes_like_serde<T: JsDecode + DeserializeOwned + PartialEq + Debug>(text: &str) {
    let reference = serde_json::from_str::<T>(text);
    let native = with_js(|ctx| {
        let parsed = ctx.json_parse(text).expect("JSON.parse");
        T::decode_js(ctx, parsed).map_err(|error| error.to_string())
    });
    match (native, reference) {
        (Ok(native), Ok(reference)) => assert_eq!(native, reference, "decoding {text}"),
        (Err(_), Err(_)) => {}
        (native, reference) => {
            panic!("decoding {text}: native {native:?} but serde_json {reference:?}")
        }
    }
}

/// The schema accepts exactly the keys serde writes.
fn assert_schema_accepts<T: TsSchema + Serialize + Debug>(value: &T) {
    let reference = serde_json::to_value(value).expect("serialize reference");
    T::validate_json_strict(&reference)
        .unwrap_or_else(|error| panic!("schema rejects {reference} for {value:?}: {error}"));
}

fn assert_contract<T>(values: &[T])
where
    T: TsSchema + JsEncode + JsDecode + Serialize + DeserializeOwned + PartialEq + Debug,
{
    for value in values {
        assert_encodes_like_serde(value);
        assert_decodes_like_serde::<T>(&serde_json::to_string(value).expect("serialize"));
        assert_schema_accepts(value);
    }
}

fn decode_error<T: JsDecode + Debug>(source: &str) -> String {
    with_js(|ctx| {
        let value = ctx
            .eval::<JsValue<'_>, _>(format!("({source})"))
            .expect("evaluate test value");
        let error = T::decode_js(ctx, value).expect_err("decoding must fail");
        error.to_string()
    })
}

fn encode_error<T: JsEncode>(value: &T) -> String {
    with_js(|ctx| {
        value
            .encode_js(ctx)
            .expect_err("encoding must fail")
            .to_string()
    })
}

// ---------------------------------------------------------------------------
// Structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Position {
    x: u8,
    y: i32,
}

fn default_region() -> String {
    String::from("arena")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct Player {
    player_id: u32,
    #[serde(rename = "displayName", alias = "nick")]
    name: String,
    nickname: Option<String>,
    #[serde(default)]
    lives: u8,
    #[serde(default = "default_region")]
    spawn_region: String,
    #[serde(skip)]
    cache: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    tags: Vec<String>,
    #[serde(skip_serializing)]
    input_only: Option<u32>,
    #[serde(skip_deserializing)]
    output_only: u32,
    position: Position,
}

fn player() -> Player {
    Player {
        player_id: 7,
        name: String::from("Nami"),
        nickname: None,
        lives: 3,
        spawn_region: String::from("docks"),
        cache: 0,
        tags: vec![String::from("captain")],
        input_only: None,
        output_only: 11,
        position: Position { x: 4, y: -2 },
    }
}

#[test]
fn named_struct_field_attributes_follow_serde() {
    let untagged = Player {
        tags: Vec::new(),
        nickname: Some(String::from("navigator")),
        ..player()
    };
    assert_contract(&[player(), untagged]);
    assert_decodes_like_serde::<Player>(
        r#"{"playerId":1,"nick":"alias","inputOnly":5,"outputOnly":9,"position":{"x":1,"y":2}}"#,
    );
    assert_decodes_like_serde::<Player>(r#"{"playerId":1,"displayName":"x"}"#);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Pair(u32, String, #[serde(skip)] u8);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Meters(f64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Marker;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(transparent)]
struct Id<T> {
    value: T,
    #[serde(skip)]
    marker: PhantomData<T>,
}

#[test]
fn tuple_newtype_unit_and_transparent_structs_follow_serde() {
    assert_contract(&[Pair(1, String::from("one"), 0)]);
    assert_contract(&[Meters(1.5), Meters(-0.25)]);
    assert_contract(&[Marker]);
    assert_contract(&[Id {
        value: String::from("user-1"),
        marker: PhantomData,
    }]);
    assert_decodes_like_serde::<Pair>("[1]");
    assert_decodes_like_serde::<Pair>(r#"[1,"a",3]"#);
    assert_eq!(Marker::ts_type(), TsType::Null);
    assert_eq!(Meters::ts_type(), TsType::Number);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Item {
    id: u32,
    label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Wrapper<T> {
    inner: T,
    count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Tree {
    label: String,
    children: Vec<Tree>,
    next: Option<Box<Tree>>,
    weights: BTreeMap<u32, f64>,
}

#[test]
fn nested_generic_and_recursive_types_follow_serde() {
    let item = |id| Item {
        id,
        label: format!("item-{id}"),
    };
    assert_contract(&[Wrapper {
        inner: vec![Some(item(1)), None, Some(item(2))],
        count: 3,
    }]);
    let leaf = Tree {
        label: String::from("leaf"),
        children: Vec::new(),
        next: None,
        weights: BTreeMap::from([(1, 0.5)]),
    };
    assert_contract(&[Tree {
        label: String::from("root"),
        children: vec![leaf.clone(), leaf.clone()],
        next: Some(Box::new(leaf)),
        weights: BTreeMap::new(),
    }]);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct Meta {
    created_by: String,
    revision: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Extra {
    note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Envelope {
    id: u32,
    #[serde(flatten)]
    meta: Meta,
    #[serde(flatten)]
    extra: Option<Extra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Open {
    name: String,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

#[test]
fn flatten_merges_like_serde() {
    let meta = Meta {
        created_by: String::from("ops"),
        revision: 2,
    };
    assert_contract(&[
        Envelope {
            id: 1,
            meta: meta.clone(),
            extra: Some(Extra {
                note: String::from("hi"),
            }),
        },
        Envelope {
            id: 2,
            meta,
            extra: None,
        },
    ]);

    let open = Open {
        name: String::from("a"),
        rest: BTreeMap::from([
            (String::from("x"), json!(1)),
            (String::from("y"), json!([true, null])),
        ]),
    };
    assert_encodes_like_serde(&open);
    assert_decodes_like_serde::<Open>(&serde_json::to_string(&open).expect("serialize"));
}

#[test]
fn a_flattened_proto_key_stays_an_own_property_both_ways() {
    let open = Open {
        name: String::from("a"),
        rest: BTreeMap::from([(String::from("__proto__"), json!({ "polluted": true }))]),
    };
    let own_key = with_js(|ctx| {
        let encoded = open.encode_js(ctx).expect("encode natively");
        let check: manivelle::js::Function<'_> = ctx
            .eval(
                "(o) => Object.getPrototypeOf(o) === Object.prototype \
                 && Object.prototype.hasOwnProperty.call(o, '__proto__') \
                 && o.polluted === undefined",
            )
            .expect("compile check");
        check.call::<_, bool>((encoded,)).expect("run check")
    });
    assert!(
        own_key,
        "the encoded `__proto__` key must be an own property"
    );

    let decoded = with_js(|ctx| {
        let parsed = ctx
            .json_parse(r#"{"name":"a","__proto__":{"polluted":true}}"#)
            .expect("JSON.parse");
        Open::decode_js(ctx, parsed).expect("decode natively")
    });
    assert_eq!(decoded, open);
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(deny_unknown_fields)]
struct Strict {
    a: u8,
    #[serde(alias = "bee")]
    b: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(default)]
struct Settings {
    volume: u8,
    name: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            volume: 5,
            name: String::from("default"),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all(serialize = "SCREAMING_SNAKE_CASE"))]
struct Directional {
    #[serde(rename(serialize = "outName", deserialize = "inName"))]
    value: u8,
    plain_field: bool,
}

#[test]
fn container_attributes_follow_serde() {
    assert_contract(&[Strict { a: 1, b: Some(2) }]);
    assert_decodes_like_serde::<Strict>(r#"{"a":1,"bee":3}"#);
    assert_decodes_like_serde::<Strict>(r#"{"a":1,"c":3}"#);

    assert_contract(&[Settings {
        volume: 9,
        name: String::from("custom"),
    }]);
    assert_decodes_like_serde::<Settings>("{}");
    assert_decodes_like_serde::<Settings>(r#"{"volume":1}"#);

    assert_encodes_like_serde(&Directional {
        value: 3,
        plain_field: true,
    });
    assert_decodes_like_serde::<Directional>(r#"{"inName":4,"plain_field":false}"#);
    assert_decodes_like_serde::<Directional>(r#"{"outName":4,"PLAIN_FIELD":false}"#);
}

// ---------------------------------------------------------------------------
// Enums
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "snake_case")]
enum Command {
    Stop,
    #[serde(alias = "go")]
    Move(Position),
    Teleport(i32, i32),
    #[serde(rename_all = "camelCase")]
    Spawn {
        entity_id: u32,
        at: Option<Position>,
    },
    #[serde(rename = "noop")]
    DoNothing,
    #[serde(skip_serializing)]
    Legacy,
    #[serde(skip_deserializing)]
    Retired(u8),
    SyncHTTP2,
}

#[test]
fn externally_tagged_enums_follow_serde() {
    assert_contract(&[
        Command::Stop,
        Command::Move(Position { x: 1, y: 2 }),
        Command::Teleport(-1, 5),
        Command::Spawn {
            entity_id: 9,
            at: None,
        },
        Command::DoNothing,
        Command::SyncHTTP2,
    ]);
    assert_encodes_like_serde(&Command::Retired(1));
    for text in [
        r#"{"go":{"x":1,"y":1}}"#,
        r#"{"stop":null}"#,
        r#""legacy""#,
        r#""retired""#,
        r#""move""#,
        r#"{"stop":null,"noop":null}"#,
    ] {
        assert_decodes_like_serde::<Command>(text);
    }
    assert!(encode_error(&Command::Legacy).contains("Command::Legacy cannot be serialized"));
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
enum Event {
    Ping,
    Moved(Position),
    Damaged { source_id: u32, amount: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind")]
enum Bag {
    Counts(BTreeMap<String, u32>),
}

#[test]
fn internally_tagged_enums_follow_serde() {
    assert_contract(&[
        Event::Ping,
        Event::Moved(Position { x: 3, y: 4 }),
        Event::Damaged {
            source_id: 2,
            amount: 1.5,
        },
    ]);
    assert_decodes_like_serde::<Event>(r#"{"kind":"Ping","ignored":true}"#);
    assert_decodes_like_serde::<Event>(r#"{"x":1,"kind":"Moved","y":2}"#);

    let bag = Bag::Counts(BTreeMap::from([(String::from("apples"), 3)]));
    assert_encodes_like_serde(&bag);
    assert_decodes_like_serde::<Bag>(&serde_json::to_string(&bag).expect("serialize"));
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "t", content = "c")]
enum Message {
    Empty,
    Text(String),
    Maybe(Option<u32>),
    Pair(u8, u8),
    Named { id: u64 },
}

#[test]
fn adjacently_tagged_enums_follow_serde() {
    assert_contract(&[
        Message::Empty,
        Message::Text(String::from("hello")),
        Message::Maybe(Some(4)),
        Message::Maybe(None),
        Message::Pair(1, 2),
        Message::Named { id: 77 },
    ]);
    for text in [
        r#"{"t":"Maybe"}"#,
        r#"{"c":null,"t":"Empty"}"#,
        r#"{"t":"Text"}"#,
        r#"{"t":"Named","c":{"id":1},"other":0}"#,
    ] {
        assert_decodes_like_serde::<Message>(text);
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(untagged)]
enum Lookup {
    Nothing,
    Id(u32),
    Coordinates(i32, i32),
    Query { name: String, limit: Option<u32> },
    Text(String),
}

#[test]
fn untagged_enums_try_variants_in_order() {
    assert_contract(&[
        Lookup::Nothing,
        Lookup::Id(3),
        Lookup::Coordinates(-1, 2),
        Lookup::Query {
            name: String::from("q"),
            limit: None,
        },
        Lookup::Text(String::from("free text")),
    ]);
    for text in [r#"{"name":"only"}"#, "-5", "[1]", "true"] {
        assert_decodes_like_serde::<Lookup>(text);
    }
}

// ---------------------------------------------------------------------------
// Codec opt-ins
// ---------------------------------------------------------------------------

/// Third-party-like type: serde impls only, no `TsSchema` and no native codec.
#[derive(Debug, Clone, PartialEq)]
struct ExternalId(u64);

impl Serialize for ExternalId {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("ext-{}", self.0))
    }
}

impl<'de> Deserialize<'de> for ExternalId {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.strip_prefix("ext-")
            .and_then(|digits| digits.parse().ok())
            .map(Self)
            .ok_or_else(|| serde::de::Error::custom("expected ext-<number>"))
    }
}

mod hex {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &u32, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&format!("{value:x}"))
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<u32, D::Error> {
        let text = String::deserialize(deserializer)?;
        u32::from_str_radix(&text, 16).map_err(serde::de::Error::custom)
    }
}

/// Native codec mirroring `shout_serde`: upper-case on the wire.
mod shout_js {
    use manivelle::js::{Ctx, Result, Value};
    use manivelle::{JsDecode, JsEncode};

    pub fn encode_js<'js>(value: &str, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        value.to_uppercase().encode_js(ctx)
    }

    pub fn decode_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String> {
        String::decode_js(ctx, value).map(|text| text.to_lowercase())
    }
}

mod shout_serde {
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S: Serializer>(value: &str, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&value.to_uppercase())
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<String, D::Error> {
        String::deserialize(deserializer).map(|text| text.to_lowercase())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Account {
    #[manivelle(codec = "json", type = "string")]
    external: ExternalId,
    #[serde(with = "hex")]
    #[manivelle(codec = "json", type = "string")]
    mask: u32,
    #[serde(with = "shout_serde")]
    #[manivelle(with = "shout_js")]
    call_sign: String,
    balance: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(into = "String", try_from = "String")]
#[manivelle(codec = "json")]
struct Email(String);

impl From<Email> for String {
    fn from(email: Email) -> Self {
        email.0
    }
}

impl TryFrom<String> for Email {
    type Error = String;

    fn try_from(text: String) -> Result<Self, Self::Error> {
        if text.contains('@') {
            Ok(Self(text))
        } else {
            Err(format!("invalid email {text}"))
        }
    }
}

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

// ---------------------------------------------------------------------------
// Error paths
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
struct Log {
    event: Event,
    lookup: Lookup,
    command: Command,
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

#[test]
fn lone_surrogates_in_names_are_rejected_not_misread() {
    let variant = decode_error::<Command>(r#""\uD800""#);
    assert!(variant.contains("lone surrogates"), "{variant}");

    let tag = decode_error::<Event>(r#"{ kind: "\uD800" }"#);
    assert!(tag.contains("lone surrogates"), "{tag}");

    let key = decode_error::<Strict>(r#"{ a: 1, "\uD800": 2 }"#);
    assert!(key.contains("lone surrogates"), "{key}");
}
