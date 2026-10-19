//! Codec opt-ins (`json`, custom `with`) and the error messages of decoding.

use manivelle::{TsSchema, TsType};

use super::harness::{assert_contract, assert_decodes_like_serde, decode_error};
use super::types::{
    Account, Command, Email, Event, ExternalId, Item, Log, Player, Strict, Wrapper,
};

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

#[test]
fn lone_surrogates_in_names_are_rejected_not_misread() {
    let variant = decode_error::<Command>(r#""\uD800""#);
    assert!(variant.contains("lone surrogates"), "{variant}");

    let tag = decode_error::<Event>(r#"{ kind: "\uD800" }"#);
    assert!(tag.contains("lone surrogates"), "{tag}");

    let key = decode_error::<Strict>(r#"{ a: 1, "\uD800": 2 }"#);
    assert!(key.contains("lone surrogates"), "{key}");
}
