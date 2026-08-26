use std::fs;

use xsync_core::protocol_v2::{decode, encode, V2CodecError};

const VECTOR_FILE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../protocol-v2-vectors/payload-v1.tsv"
);

#[test]
fn corpus_valid_vectors_decode() {
    let mut count = 0;
    for line in fs::read_to_string(VECTOR_FILE).unwrap().lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        assert_eq!(fields.len(), 5, "malformed vector line: {line}");
        if fields[1] == "malformed" {
            continue;
        }
        let message_type: u8 = fields[2].parse().unwrap();
        let payload = hex(fields[3]);
        let message = decode(message_type, &payload)
            .unwrap_or_else(|error| panic!("{} failed to decode: {error}", fields[0]));
        assert_eq!(
            encode(&message).unwrap(),
            payload,
            "{} changed during decode/re-encode",
            fields[0]
        );
        count += 1;
    }
    assert_eq!(count, 8);
}

#[test]
fn corpus_malformed_vectors_fail_closed() {
    let mut count = 0;
    for line in fs::read_to_string(VECTOR_FILE).unwrap().lines() {
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        let fields: Vec<_> = line.split('\t').collect();
        if fields[1] != "malformed" {
            continue;
        }
        let message_type: u8 = fields[2].parse().unwrap();
        let payload = hex(fields[3]);
        let error =
            decode(message_type, &payload).expect_err(&format!("{} was accepted", fields[0]));
        let expected = match fields[4] {
            "invalid message type" => "message type",
            "truncated path" => "truncated",
            "invalid boolean" => "boolean",
            "invalid stat status enum" => "stat status",
            "missing response with digest present" => "stat response",
            "trailing payload byte" => "trailing",
            "path length exceeds 1 MiB" => "path",
            _ => panic!("unknown malformed rule for {}", fields[0]),
        };
        assert!(
            error.to_string().contains(expected),
            "{} reported {error}, expected rule {expected}",
            fields[0]
        );
        count += 1;
    }
    assert_eq!(count, 7);
}

#[test]
fn boundary_vectors_cover_maximum_path_and_collection() {
    use xsync_core::protocol_v2::{encode, BrowseEntry, V2Message};

    let max_path = vec![0xa5; 1024 * 1024];
    let payload = encode(&V2Message::ListRequest {
        path: max_path.clone(),
        page_token: 0,
        page_size: 65_536,
    })
    .unwrap();
    assert_eq!(
        decode(14, &payload).unwrap(),
        V2Message::ListRequest {
            path: max_path,
            page_token: 0,
            page_size: 65_536,
        }
    );

    let entry = BrowseEntry {
        name: Vec::new(),
        kind: 1,
        size: 0,
        mtime_ns: 0,
        mode: 0,
        symlink_target: Vec::new(),
    };
    let entries = vec![entry; 65_536];
    let payload = encode(&V2Message::ListPage {
        related_id: 1,
        page_token: 0,
        final_page: true,
        entries: entries.clone(),
    })
    .unwrap();
    assert_eq!(
        decode(15, &payload).unwrap(),
        V2Message::ListPage {
            related_id: 1,
            page_token: 0,
            final_page: true,
            entries,
        }
    );
}

#[test]
fn malformed_corpus_names_specific_codec_errors() {
    let lines = fs::read_to_string(VECTOR_FILE).unwrap();
    let line = lines
        .lines()
        .find(|line| line.starts_with("truncated-path\t"))
        .unwrap();
    let fields: Vec<_> = line.split('\t').collect();
    assert_eq!(
        decode(fields[2].parse().unwrap(), &hex(fields[3])),
        Err(V2CodecError::Truncated)
    );
}

fn hex(value: &str) -> Vec<u8> {
    assert!(value.len() % 2 == 0);
    (0..value.len())
        .step_by(2)
        .map(|index| u8::from_str_radix(&value[index..index + 2], 16).unwrap())
        .collect()
}
