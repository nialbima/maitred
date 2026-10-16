//! The test kit's own reader and writer of maitred's ticket file, from ARCHITECTURE.md's byte layout ("The
//! ticket: two slots and a lock"). It shares no code with maitred's store, so the two can disagree, and a test
//! that runs both over the same files shows where.

use maitred_scenario::Class;

/// One slot's size; a ticket file holds two.
pub const SLOT: usize = 4096;
/// The fixed header's size; the payload starts here.
pub const HEADER: usize = 96;
/// The most payload a slot holds.
pub const MAX_PAYLOAD: usize = 4000;
const MAGIC: &[u8; 8] = b"MTRDTICK";
/// The major version this reader understands. A ticket of a newer major reads as unknown (ARCHITECTURE.md,
/// Versions: Nick's decision on MT-011); a newer minor is still read from its header.
pub const MAJOR: u8 = 4;

/// A slot's fixed header, field by field as the doc lists them, with the codes as stored.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Header {
    pub generation: u64,
    pub major: u8,
    pub minor: u8,
    pub class: u8,
    pub state: u8,
    pub cpu_millicores: u32,
    pub mem_mb: u32,
    pub sid: i32,
    pub seq: u64,
    /// Milliseconds on the boot clock; 0 means never.
    pub last_pass: u64,
    pub admitted_at: u64,
    pub recreated_at: u64,
    /// The platform's own start time for the holder, compared only for equality.
    pub holder_start: u64,
    pub pgid: i32,
    pub holder_pid: i32,
    /// The first 8 bytes of SHA-256 over the boot id and pid namespace.
    pub boot: u64,
}

/// A ticket's state as a reader takes it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StateCode {
    Waiting,
    Admitted,
    Running,
}

impl Header {
    /// The class. If either code is one this reader doesn't know, the whole ticket reads as heavy and running (the
    /// doc's rule for a newer minor), so it is charged rather than queued.
    #[must_use]
    pub fn class(&self) -> Class {
        if !self.codes_known() {
            return Class::Heavy;
        }
        match self.class {
            0 => Class::Light,
            2 => Class::Quiet,
            _ => Class::Heavy,
        }
    }

    /// Whether both the class and the state code are ones this reader knows.
    #[must_use]
    pub fn codes_known(&self) -> bool {
        self.class <= 2 && self.state <= 2
    }

    /// The state; running if either code is one this reader doesn't know (see [`Header::class`]).
    #[must_use]
    pub fn state(&self) -> StateCode {
        if !self.codes_known() {
            return StateCode::Running;
        }
        match self.state {
            0 => StateCode::Waiting,
            1 => StateCode::Admitted,
            _ => StateCode::Running,
        }
    }
}

/// Why a slot can't be used.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SlotError {
    Magic,
    Length(u32),
    Checksum,
}

/// What a reader makes of a whole ticket file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Read {
    /// The newest slot that passed its checks.
    /// The payload is kept as bytes: a slot is valid on its checksum, and a reader that can't parse the
    /// payload still places and charges the ticket from its header (Versions).
    Valid { header: Header, payload: Vec<u8> },
    /// Neither slot passed, or the file isn't two slots long.
    Unknown,
}

impl Read {
    #[must_use]
    pub fn generation(&self) -> Option<u64> {
        match self {
            Read::Valid { header, .. } => Some(header.generation),
            Read::Unknown => None,
        }
    }

    #[must_use]
    /// The payload as text, or `None` if there is no valid slot or the payload isn't UTF-8.
    pub fn payload(&self) -> Option<&str> {
        match self {
            Read::Valid { payload, .. } => std::str::from_utf8(payload).ok(),
            Read::Unknown => None,
        }
    }
}

/// IEEE CRC-32 (reflected, polynomial 0xEDB88320), the checksum the doc names. Written here rather than taken
/// from the crate maitred uses, so the two are checked against each other.
#[must_use]
pub fn crc32(bytes: &[u8]) -> u32 {
    let mut crc = !0_u32;
    for byte in bytes {
        crc ^= u32::from(*byte);
        for _ in 0..8 {
            crc = if crc & 1 == 1 {
                (crc >> 1) ^ 0xEDB8_8320
            } else {
                crc >> 1
            };
        }
    }
    !crc
}

fn u32_at(slot: &[u8], at: usize) -> u32 {
    u32::from_le_bytes(slot[at..at + 4].try_into().expect("4 bytes"))
}

fn i32_at(slot: &[u8], at: usize) -> i32 {
    i32::from_le_bytes(slot[at..at + 4].try_into().expect("4 bytes"))
}

fn u64_at(slot: &[u8], at: usize) -> u64 {
    u64::from_le_bytes(slot[at..at + 8].try_into().expect("8 bytes"))
}

/// Decodes one slot, checking its magic, its length and its checksum.
///
/// # Errors
///
/// Returns why the slot can't be used.
pub fn decode_slot(slot: &[u8; SLOT]) -> Result<(Header, Vec<u8>), SlotError> {
    if &slot[0..8] != MAGIC {
        return Err(SlotError::Magic);
    }
    let length = u32_at(slot, 12);
    let payload_len = usize::try_from(length).unwrap_or(usize::MAX);
    if payload_len > MAX_PAYLOAD {
        return Err(SlotError::Length(length));
    }
    let end = HEADER + payload_len;
    if crc32(&slot[12..end]) != u32_at(slot, 8) {
        return Err(SlotError::Checksum);
    }
    let header = Header {
        generation: u64_at(slot, 16),
        major: slot[24],
        minor: slot[25],
        class: slot[26],
        state: slot[27],
        cpu_millicores: u32_at(slot, 28),
        mem_mb: u32_at(slot, 32),
        sid: i32_at(slot, 36),
        seq: u64_at(slot, 40),
        last_pass: u64_at(slot, 48),
        admitted_at: u64_at(slot, 56),
        recreated_at: u64_at(slot, 64),
        holder_start: u64_at(slot, 72),
        pgid: i32_at(slot, 80),
        holder_pid: i32_at(slot, 84),
        boot: u64_at(slot, 88),
    };
    Ok((header, slot[HEADER..end].to_vec()))
}

/// Encodes one slot: the test kit's own writer, for building tickets and torn states in tests and faults.
///
/// # Panics
///
/// If the payload is over 4000 bytes, which no valid slot holds.
#[must_use]
pub fn encode_slot(header: &Header, payload: &str) -> [u8; SLOT] {
    assert!(
        payload.len() <= MAX_PAYLOAD,
        "a payload of {} bytes doesn't fit a slot",
        payload.len()
    );
    let mut slot = [0_u8; SLOT];
    slot[0..8].copy_from_slice(MAGIC);
    slot[12..16].copy_from_slice(
        &u32::try_from(payload.len())
            .expect("at most 4000")
            .to_le_bytes(),
    );
    slot[16..24].copy_from_slice(&header.generation.to_le_bytes());
    slot[24] = header.major;
    slot[25] = header.minor;
    slot[26] = header.class;
    slot[27] = header.state;
    slot[28..32].copy_from_slice(&header.cpu_millicores.to_le_bytes());
    slot[32..36].copy_from_slice(&header.mem_mb.to_le_bytes());
    slot[36..40].copy_from_slice(&header.sid.to_le_bytes());
    slot[40..48].copy_from_slice(&header.seq.to_le_bytes());
    slot[48..56].copy_from_slice(&header.last_pass.to_le_bytes());
    slot[56..64].copy_from_slice(&header.admitted_at.to_le_bytes());
    slot[64..72].copy_from_slice(&header.recreated_at.to_le_bytes());
    slot[72..80].copy_from_slice(&header.holder_start.to_le_bytes());
    slot[80..84].copy_from_slice(&header.pgid.to_le_bytes());
    slot[84..88].copy_from_slice(&header.holder_pid.to_le_bytes());
    slot[88..96].copy_from_slice(&header.boot.to_le_bytes());
    slot[HEADER..HEADER + payload.len()].copy_from_slice(payload.as_bytes());
    let crc = crc32(&slot[12..HEADER + payload.len()]);
    slot[8..12].copy_from_slice(&crc.to_le_bytes());
    slot
}

/// Reads a whole ticket file as the doc says: keep the slots that pass, take the higher generation. With two valid
/// slots of one generation, which no correct writer leaves, the second is read. The doc's "read again, up to
/// three times" is the caller's: this reads the bytes it is given once. If the newest valid slot is of a newer
/// major version, the ticket is unknown.
#[must_use]
pub fn read_ticket(bytes: &[u8]) -> Read {
    if bytes.len() != 2 * SLOT {
        return Read::Unknown;
    }
    bytes
        .as_chunks::<SLOT>()
        .0
        .iter()
        .filter_map(|slot| decode_slot(slot).ok())
        .max_by_key(|(header, _)| header.generation)
        .filter(|(header, _)| header.major <= MAJOR)
        .map_or(Read::Unknown, |(header, payload)| Read::Valid {
            header,
            payload,
        })
}
