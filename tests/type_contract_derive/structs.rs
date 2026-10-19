//! Struct shapes: named fields, tuples, newtypes, generics, flatten and container attributes.

use std::collections::BTreeMap;
use std::marker::PhantomData;

use manivelle::js::Function;
use manivelle::{JsDecode, JsEncode, TsSchema, TsType};
use serde_json::json;

use super::harness::{
    assert_contract, assert_decodes_like_serde, assert_encodes_like_serde, with_js,
};
use super::types::{
    Directional, Envelope, Extra, Id, Item, Marker, Meta, Meters, Open, Pair, Player, Position,
    Settings, Strict, Tree, Wrapper,
};

#[test]
fn named_struct_field_attributes_follow_serde() {
    let player = Player {
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
    };
    let untagged = Player {
        tags: Vec::new(),
        nickname: Some(String::from("navigator")),
        ..player.clone()
    };
    assert_contract(&[player, untagged]);
    assert_decodes_like_serde::<Player>(
        r#"{"playerId":1,"nick":"alias","inputOnly":5,"outputOnly":9,"position":{"x":1,"y":2}}"#,
    );
    assert_decodes_like_serde::<Player>(r#"{"playerId":1,"displayName":"x"}"#);
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
        let check: Function<'_> = ctx
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
