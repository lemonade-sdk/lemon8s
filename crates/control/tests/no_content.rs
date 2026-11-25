use control::{KeyHash, Name};
use policy::ProvenanceRecord;
use serde_json::json;

const HASH: &str = "9f86d081884c7d659a2feaa0c55ad015a3bf4f1b2b0b822cd15d6c15b0f00a08";

#[test]
fn a_name_cannot_carry_free_text() {
    for text in [
        "Summarize the attached contract",
        "line\nbreak",
        "quote\"",
        "",
        "{\"role\":\"user\"}",
        "ümlaut",
    ] {
        assert!(Name::new(text).is_err(), "{text:?} accepted");
    }
    assert!(Name::new("a".repeat(Name::MAX_LEN + 1)).is_err());
    for id in [
        "unsloth/Qwen3-30B-A3B-Instruct-2507-GGUF:Q4_0",
        "Qwen3-30B-A3B-Instruct-2507-GGUF",
        "finance",
        "engine_1",
    ] {
        assert!(Name::new(id).is_ok(), "{id:?} refused");
    }
}

#[test]
fn names_parsed_from_a_request_body_refuse_free_text() {
    for text in [
        "please write my essay on the merger",
        "a secret project codename",
    ] {
        assert!(
            serde_json::from_value::<Name>(json!(text)).is_err(),
            "{text:?} accepted"
        );
        assert!(serde_json::from_value::<Vec<Name>>(json!(["finance", text])).is_err());
    }
    assert_eq!(
        serde_json::from_value::<Vec<Name>>(json!(["finance", "legal"])).unwrap(),
        [Name::new("finance").unwrap(), Name::new("legal").unwrap()]
    );
}

#[test]
fn key_hashes_are_fixed_width_lowercase_hex() {
    let hash: KeyHash = serde_json::from_value(json!(HASH)).unwrap();
    assert_eq!(serde_json::to_value(hash).unwrap(), json!(HASH));
    assert!(serde_json::from_value::<KeyHash>(json!(HASH.to_uppercase())).is_err());
    assert!(serde_json::from_value::<KeyHash>(json!(&HASH[..62])).is_err());
}

#[test]
fn provenance_records_parse_strictly() {
    let record = json!({
        "seq": 3,
        "epoch": 2,
        "actor": "alice",
        "session": 0,
        "label": ["finance"],
        "event": {"kind": "request_started", "request": 4},
        "denial": null,
    });
    assert!(serde_json::from_value::<ProvenanceRecord>(record.clone()).is_ok());
    let mut extra = record;
    extra["event"]["prompt"] = json!("hi");
    assert!(serde_json::from_value::<ProvenanceRecord>(extra).is_err());
}
