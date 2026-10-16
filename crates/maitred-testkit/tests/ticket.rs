//! The test kit's own ticket reader, written from ARCHITECTURE.md's byte layout ("The ticket: two slots and a
//! lock"). These are the Test kit section's reader row: every torn state, built directly.

use maitred_testkit::ticket::{
    Header, Read, SLOT, SlotError, StateCode, crc32, decode_slot, encode_slot, read_ticket,
};

fn header(generation: u64) -> Header {
    Header {
        generation,
        major: 4,
        minor: 0,
        class: 1,
        state: 2,
        cpu_millicores: 2_000,
        mem_mb: 1_200,
        sid: 48_210,
        seq: 4_183,
        last_pass: 120_000,
        admitted_at: 118_000,
        recreated_at: 0,
        holder_start: 987_654,
        pgid: 48_211,
        holder_pid: 48_210,
        boot: 0x0123_4567_89ab_cdef,
    }
}

fn file(older: &[u8; SLOT], newer: &[u8; SLOT]) -> Vec<u8> {
    let mut bytes = older.to_vec();
    bytes.extend_from_slice(newer);
    bytes
}

#[test]
fn the_checksum_is_ieee_crc_32() {
    // The standard check value for IEEE CRC-32.
    assert_eq!(crc32(b"123456789"), 0xCBF4_3926);
}

#[test]
fn a_slot_built_byte_by_byte_from_the_doc_decodes_to_its_fields() {
    let payload = br#"{"v":"4.0"}"#;
    let mut slot = [0_u8; SLOT];
    slot[0..8].copy_from_slice(b"MTRDTICK");
    slot[12..16].copy_from_slice(&u32::try_from(payload.len()).unwrap().to_le_bytes());
    slot[16..24].copy_from_slice(&7_u64.to_le_bytes());
    slot[24] = 4;
    slot[25] = 0;
    slot[26] = 2;
    slot[27] = 1;
    slot[28..32].copy_from_slice(&500_u32.to_le_bytes());
    slot[32..36].copy_from_slice(&256_u32.to_le_bytes());
    slot[36..40].copy_from_slice(&(-1_i32).to_le_bytes());
    slot[40..48].copy_from_slice(&99_u64.to_le_bytes());
    slot[48..56].copy_from_slice(&1_000_u64.to_le_bytes());
    slot[80..84].copy_from_slice(&(-2_i32).to_le_bytes());
    slot[84..88].copy_from_slice(&(-3_i32).to_le_bytes());
    slot[88..96].copy_from_slice(&42_u64.to_le_bytes());
    slot[96..96 + payload.len()].copy_from_slice(payload);
    let crc = crc32(&slot[12..96 + payload.len()]);
    slot[8..12].copy_from_slice(&crc.to_le_bytes());

    let (header, body) = decode_slot(&slot).expect("a well-formed slot");

    assert_eq!(
        (
            header.generation,
            header.class,
            header.state,
            header.cpu_millicores,
            header.mem_mb
        ),
        (7, 2, 1, 500, 256)
    );
    assert_eq!(
        (
            header.sid,
            header.seq,
            header.last_pass,
            header.pgid,
            header.holder_pid
        ),
        (-1, 99, 1_000, -2, -3)
    );
    assert_eq!(header.boot, 42);
    assert_eq!(body, r#"{"v":"4.0"}"#);
}

#[test]
fn encoding_then_decoding_gives_back_the_same_slot() {
    let slot = encode_slot(&header(9), r#"{"label":"a"}"#);

    assert_eq!(
        decode_slot(&slot),
        Ok((header(9), r#"{"label":"a"}"#.to_string()))
    );
}

#[test]
fn a_slot_with_the_wrong_magic_fails() {
    let mut slot = encode_slot(&header(1), "{}");
    slot[0] = b'X';

    assert_eq!(decode_slot(&slot), Err(SlotError::Magic));
}

#[test]
fn a_slot_whose_length_is_over_4000_fails() {
    let mut slot = encode_slot(&header(1), "{}");
    slot[12..16].copy_from_slice(&4_001_u32.to_le_bytes());

    assert_eq!(decode_slot(&slot), Err(SlotError::Length(4_001)));
}

#[test]
fn a_bad_checksum_fails_even_when_the_payload_is_valid_json() {
    let mut slot = encode_slot(&header(1), r#"{"v":"4.0"}"#);
    slot[8] ^= 0xFF;

    assert_eq!(decode_slot(&slot), Err(SlotError::Checksum));
}

#[test]
fn a_change_anywhere_the_checksum_covers_fails_the_slot() {
    let clean = encode_slot(&header(1), r#"{"v":"4.0"}"#);
    let covered_end = 96 + r#"{"v":"4.0"}"#.len();
    for offset in 12..covered_end {
        let mut slot = clean;
        slot[offset] ^= 0x01;
        assert!(
            decode_slot(&slot).is_err(),
            "a flip at byte {offset} went unnoticed"
        );
    }
}

#[test]
fn a_change_in_the_padding_doesnt_fail_the_slot() {
    let mut slot = encode_slot(&header(1), "{}");
    slot[SLOT - 1] = 0xFF;

    assert!(decode_slot(&slot).is_ok());
}

#[test]
fn the_reader_takes_the_higher_of_two_valid_generations_whichever_slot_holds_it() {
    let older = encode_slot(&header(4), "{}");
    let newer = encode_slot(&header(5), "{}");

    assert_eq!(read_ticket(&file(&older, &newer)).generation(), Some(5));
    assert_eq!(read_ticket(&file(&newer, &older)).generation(), Some(5));
}

#[test]
fn a_new_slot_torn_at_any_length_never_hides_the_old_one_or_is_read_half_written() {
    // The owner writes generation 6 over the slot that held generation 4; generation 5 stays whole.
    let old = encode_slot(&header(4), r#"{"label":"old"}"#);
    let kept = encode_slot(&header(5), r#"{"label":"kept"}"#);
    let new = encode_slot(&header(6), r#"{"label":"new"}"#);
    for torn_at in 0..=SLOT {
        let mut slot = old;
        slot[..torn_at].copy_from_slice(&new[..torn_at]);
        let read = read_ticket(&file(&slot, &kept));
        let generation = read.generation();
        assert!(
            generation == Some(5) || generation == Some(6),
            "torn at {torn_at}: read generation {generation:?}"
        );
        if generation == Some(6) {
            assert_eq!(
                read.payload(),
                Some(r#"{"label":"new"}"#),
                "torn at {torn_at}: half-written read"
            );
        }
    }
}

#[test]
fn a_ticket_with_no_valid_slot_is_unknown() {
    let mut first = encode_slot(&header(1), "{}");
    let mut second = encode_slot(&header(2), "{}");
    first[8] ^= 0xFF;
    second[8] ^= 0xFF;

    assert_eq!(read_ticket(&file(&first, &second)), Read::Unknown);
}

#[test]
fn a_ticket_file_of_the_wrong_size_is_unknown() {
    assert_eq!(read_ticket(&[0_u8; 100]), Read::Unknown);
}

#[test]
fn an_unknown_state_code_reads_as_running_and_a_known_one_as_itself() {
    let mut unknown = header(1);
    unknown.state = 9;

    assert_eq!(unknown.state(), StateCode::Running);
    assert_eq!(header(1).state(), StateCode::Running);
    assert_eq!(
        Header {
            state: 0,
            ..header(1)
        }
        .state(),
        StateCode::Waiting
    );
}

#[test]
fn an_unknown_class_code_reads_as_heavy() {
    let mut unknown = header(1);
    unknown.class = 7;

    assert_eq!(unknown.class(), maitred_scenario::Class::Heavy);
    assert_eq!(
        Header {
            class: 2,
            ..header(1)
        }
        .class(),
        maitred_scenario::Class::Quiet
    );
}
