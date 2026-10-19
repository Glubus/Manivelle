//! The types the `type_contract_derive` tests check: each derives serde and `TsSchema`.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use manivelle::TsSchema;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Position {
    pub(crate) x: u8,
    pub(crate) y: i32,
}

pub(crate) fn default_region() -> String {
    String::from("arena")
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Player {
    pub(crate) player_id: u32,
    #[serde(rename = "displayName", alias = "nick")]
    pub(crate) name: String,
    pub(crate) nickname: Option<String>,
    #[serde(default)]
    pub(crate) lives: u8,
    #[serde(default = "default_region")]
    pub(crate) spawn_region: String,
    #[serde(skip)]
    pub(crate) cache: u64,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub(crate) tags: Vec<String>,
    #[serde(skip_serializing)]
    pub(crate) input_only: Option<u32>,
    #[serde(skip_deserializing)]
    pub(crate) output_only: u32,
    pub(crate) position: Position,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Pair(
    pub(crate) u32,
    pub(crate) String,
    #[serde(skip)] pub(crate) u8,
);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Meters(pub(crate) f64);

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Marker;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(transparent)]
pub(crate) struct Id<T> {
    pub(crate) value: T,
    #[serde(skip)]
    pub(crate) marker: PhantomData<T>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Item {
    pub(crate) id: u32,
    pub(crate) label: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Wrapper<T> {
    pub(crate) inner: T,
    pub(crate) count: usize,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Tree {
    pub(crate) label: String,
    pub(crate) children: Vec<Tree>,
    pub(crate) next: Option<Box<Tree>>,
    pub(crate) weights: BTreeMap<u32, f64>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Meta {
    pub(crate) created_by: String,
    pub(crate) revision: u32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Extra {
    pub(crate) note: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Envelope {
    pub(crate) id: u32,
    #[serde(flatten)]
    pub(crate) meta: Meta,
    #[serde(flatten)]
    pub(crate) extra: Option<Extra>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Open {
    pub(crate) name: String,
    #[serde(flatten)]
    pub(crate) rest: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(deny_unknown_fields)]
pub(crate) struct Strict {
    pub(crate) a: u8,
    #[serde(alias = "bee")]
    pub(crate) b: Option<u8>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(default)]
pub(crate) struct Settings {
    pub(crate) volume: u8,
    pub(crate) name: String,
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
pub(crate) struct Directional {
    #[serde(rename(serialize = "outName", deserialize = "inName"))]
    pub(crate) value: u8,
    pub(crate) plain_field: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "snake_case")]
pub(crate) enum Command {
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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind", rename_all_fields = "camelCase")]
pub(crate) enum Event {
    Ping,
    Moved(Position),
    Damaged { source_id: u32, amount: f32 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind")]
pub(crate) enum Bag {
    Counts(BTreeMap<String, u32>),
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(tag = "t", content = "c")]
pub(crate) enum Message {
    Empty,
    Text(String),
    Maybe(Option<u32>),
    Pair(u8, u8),
    Named { id: u64 },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(untagged)]
pub(crate) enum Lookup {
    Nothing,
    Id(u32),
    Coordinates(i32, i32),
    Query { name: String, limit: Option<u32> },
    Text(String),
}

/// Third-party-like type: serde impls only, no `TsSchema` and no native codec.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct ExternalId(pub(crate) u64);

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
    use manivelle::js::{Ctx, Value};
    use manivelle::{CodecError, JsDecode, JsEncode};

    pub fn encode_js<'js>(value: &str, ctx: &Ctx<'js>) -> Result<Value<'js>, CodecError> {
        value.to_uppercase().encode_js(ctx)
    }

    pub fn decode_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<String, CodecError> {
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
pub(crate) struct Account {
    #[manivelle(codec = "json", type = "string")]
    pub(crate) external: ExternalId,
    #[serde(with = "hex")]
    #[manivelle(codec = "json", type = "string")]
    pub(crate) mask: u32,
    #[serde(with = "shout_serde")]
    #[manivelle(with = "shout_js")]
    pub(crate) call_sign: String,
    pub(crate) balance: i64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(into = "String", try_from = "String")]
#[manivelle(codec = "json")]
pub(crate) struct Email(pub(crate) String);

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

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
pub(crate) struct Log {
    pub(crate) event: Event,
    pub(crate) lookup: Lookup,
    pub(crate) command: Command,
}
