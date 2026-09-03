use std::fs;

use xsync_core::protocol_v3::{decode, encode, message_type, V3CodecError};

const VECTOR_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-v3-vectors/payload-v1.tsv"
);

fn rows() -> Vec<Vec<String>> {
    fs::read_to_string(VECTOR_FILE)
        .unwrap()
        .lines()
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .map(|line| {
            let fields: Vec<String> = line.split('\t').map(str::to_owned).collect();
            assert_eq!(fields.len(), 5, "malformed vector line: {line}");
            fields
        })
        .collect()
}

#[test]
fn corpus_valid_vectors_decode_and_re_encode_byte_exact() {
    let mut count = 0;
    for fields in rows() {
        if fields[1] == "malformed" {
            continue;
        }
        let message_type_byte: u8 = fields[2].parse().unwrap();
        let payload = hex(&fields[3]);
        let message = decode(message_type_byte, &payload)
            .unwrap_or_else(|error| panic!("{} failed to decode: {error}", fields[0]));
        assert_eq!(
            message_type(&message),
            message_type_byte,
            "{} decoded to a different type",
            fields[0]
        );
        assert_eq!(
            encode(&message).unwrap(),
            payload,
            "{} changed during decode/re-encode",
            fields[0]
        );
        count += 1;
    }
    assert_eq!(count, 22);
}

#[test]
fn corpus_malformed_vectors_fail_closed() {
    let mut count = 0;
    for fields in rows() {
        if fields[1] != "malformed" {
            continue;
        }
        let message_type_byte: u8 = fields[2].parse().unwrap();
        let payload = hex(&fields[3]);
        let error =
            decode(message_type_byte, &payload).expect_err(&format!("{} was accepted", fields[0]));
        let expected = match fields[4].as_str() {
            "invalid message type" => "message type",
            "unknown open flag" | "open flags inconsistent" => "open flags",
            "read length out of range" => "read length",
            "unknown attrs presence bit" => "presence",
            "symlink target on non-symlink" => "symlink target",
            "writability inconsistent with reason" => "reason",
            "invalid error code" => "error code",
            "trailing payload byte" => "trailing",
            "truncated attrs" => "truncated",
            "stat target inconsistent" => "stat target",
            "page size out of range" => "max entries",
            other => panic!("unknown malformed rule for {}: {other}", fields[0]),
        };
        assert!(
            error.to_string().contains(expected),
            "{} reported {error}, expected rule {expected}",
            fields[0]
        );
        count += 1;
    }
    assert_eq!(count, 12);
}

#[test]
fn corpus_covers_every_phase_one_type_once() {
    use std::collections::BTreeSet;
    let valid: BTreeSet<u8> = rows()
        .iter()
        .filter(|fields| fields[1] != "malformed")
        .map(|fields| fields[2].parse().unwrap())
        .collect();
    let expected: BTreeSet<u8> = [
        18, 42, 43, 50, 51, 56, 57, 58, 59, 60, 61, 62, 63, 80, 81, 82, 83, 84, 85, 121, 122,
    ]
    .into_iter()
    .collect();
    assert_eq!(valid, expected);
}

#[test]
fn boundary_vectors_cover_maximum_data_and_collection() {
    use xsync_core::protocol_v3::{Attrs, DirEntry, V3Message};

    let data = vec![0xa5; 8 * 1024 * 1024];
    let payload = encode(&V3Message::Write {
        handle: 1,
        offset: 0,
        digest: None,
        data: data.clone(),
    })
    .unwrap();
    assert_eq!(
        decode(61, &payload).unwrap(),
        V3Message::Write {
            handle: 1,
            offset: 0,
            digest: None,
            data,
        }
    );

    let entry = DirEntry {
        name: Vec::new(),
        attrs: Attrs::minimal(1, 0, 0, 0, [0; 16]),
    };
    let entries = vec![entry; 65_536];
    let payload = encode(&V3Message::DirPage {
        related_id: 1,
        cursor: 0,
        final_page: true,
        entries: entries.clone(),
    })
    .unwrap();
    assert_eq!(
        decode(83, &payload).unwrap(),
        V3Message::DirPage {
            related_id: 1,
            cursor: 0,
            final_page: true,
            entries,
        }
    );
}

#[test]
fn malformed_corpus_names_specific_codec_errors() {
    let fields = rows()
        .into_iter()
        .find(|fields| fields[0] == "truncated-attrs")
        .unwrap();
    assert_eq!(
        decode(fields[2].parse().unwrap(), &hex(&fields[3])),
        Err(V3CodecError::Truncated)
    );
}

fn hex(value: &str) -> Vec<u8> {
    assert!(value.len().is_multiple_of(2), "odd hex length: {value}");
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
        .collect()
}
