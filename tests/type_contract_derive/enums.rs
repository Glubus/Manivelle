//! Enum representations: externally, internally, adjacently and untagged.

use std::collections::BTreeMap;

use super::harness::{
    assert_contract, assert_decodes_like_serde, assert_encodes_like_serde, encode_error,
};
use super::types::{Bag, Command, Event, Lookup, Message, Position};

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
