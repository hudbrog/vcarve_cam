//! Paged binary scene payload and the worker message framing.
//!
//! The experiment transports a scene as one small JSON metadata document plus
//! one binary payload. Motion geometry and stock cells never travel as JSON:
//! the payload is sectioned so the UI can hand a page straight to the GPU
//! without decoding it into per-vertex structures first.
//!
//! ```text
//! magic "GUI1FRM1" | u32 section count | 24-byte records | padded sections
//! record: kind u8 | pad 7 | offset u64 LE | length u64 LE   (offset from 0)
//! ```
use serde::{Deserialize, Serialize};
use std::ops::Range;

pub const MAGIC: &[u8; 8] = b"GUI1FRM1";
pub const VERTEX_BYTES: usize = 28;
pub const PAGE_MOTIONS: usize = 8_192;
pub const PAGE_BYTES: usize = PAGE_MOTIONS * 2 * VERTEX_BYTES;
pub const RECORD_BYTES: usize = 24;
pub const ALIGN: usize = 16;

pub const SECTION_CONTOUR: u8 = 1;
pub const SECTION_MOTIONS: u8 = 2;
pub const SECTION_STOCK: u8 = 3;
pub const SECTION_SIM: u8 = 4;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Section {
    pub kind: u8,
    pub offset: usize,
    pub len: usize,
}

/// Deterministic payload assembly. Sections are appended in call order and
/// padded, so identical inputs produce identical bytes and hashes.
#[derive(Default)]
pub struct Builder {
    sections: Vec<(u8, Vec<u8>)>,
}

impl Builder {
    pub fn new() -> Self {
        Self::default()
    }
    pub fn push(&mut self, kind: u8, bytes: Vec<u8>) -> &mut Self {
        self.sections.push((kind, bytes));
        self
    }
    pub fn finish(self) -> Vec<u8> {
        let header = 8 + 4 + RECORD_BYTES * self.sections.len();
        let mut out =
            Vec::with_capacity(header + self.sections.iter().map(|s| s.1.len()).sum::<usize>());
        out.extend_from_slice(MAGIC);
        out.extend_from_slice(&(self.sections.len() as u32).to_le_bytes());
        let mut offset = header.div_ceil(ALIGN) * ALIGN;
        let mut records = Vec::with_capacity(self.sections.len());
        for (kind, bytes) in &self.sections {
            records.push((*kind, offset, bytes.len()));
            offset += bytes.len().div_ceil(ALIGN) * ALIGN;
        }
        for (kind, offset, len) in &records {
            out.push(*kind);
            out.extend_from_slice(&[0u8; 7]);
            out.extend_from_slice(&(*offset as u64).to_le_bytes());
            out.extend_from_slice(&(*len as u64).to_le_bytes());
        }
        out.resize(header.div_ceil(ALIGN) * ALIGN, 0);
        for (i, (_, bytes)) in self.sections.iter().enumerate() {
            let (_, offset, _) = records[i];
            out.resize(offset, 0);
            out.extend_from_slice(bytes);
            out.resize(offset + bytes.len().div_ceil(ALIGN) * ALIGN, 0);
        }
        out
    }
}

pub struct Payload {
    bytes: Vec<u8>,
    sections: Vec<Section>,
}

impl Payload {
    pub fn parse(bytes: Vec<u8>) -> Result<Self, String> {
        if bytes.len() < 12 || &bytes[0..8] != MAGIC {
            return Err("Payload is not a GUI1 frame".into());
        }
        let count = u32::from_le_bytes(bytes[8..12].try_into().unwrap()) as usize;
        if 12 + count * RECORD_BYTES > bytes.len() {
            return Err("Payload section table is truncated".into());
        }
        let mut sections = Vec::with_capacity(count);
        for i in 0..count {
            let at = 12 + i * RECORD_BYTES;
            let kind = bytes[at];
            let offset = u64::from_le_bytes(bytes[at + 8..at + 16].try_into().unwrap()) as usize;
            let len = u64::from_le_bytes(bytes[at + 16..at + 24].try_into().unwrap()) as usize;
            if offset
                .checked_add(len)
                .is_none_or(|end| end > bytes.len() || !offset.is_multiple_of(ALIGN))
            {
                return Err("Payload section is out of bounds or misaligned".into());
            }
            sections.push(Section { kind, offset, len });
        }
        Ok(Self { bytes, sections })
    }
    pub fn bytes(&self) -> &[u8] {
        &self.bytes
    }
    pub fn into_bytes(self) -> Vec<u8> {
        self.bytes
    }
    pub fn sections(&self) -> &[Section] {
        &self.sections
    }
    pub fn slice(&self, section: &Section) -> &[u8] {
        &self.bytes[section.offset..section.offset + section.len]
    }
    pub fn find(&self, kind: u8, index: usize) -> Option<&Section> {
        self.sections.iter().filter(|s| s.kind == kind).nth(index)
    }
}

/// Integer page table for the motion section of a payload.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PageTable {
    pub motions: usize,
    pub page_motions: usize,
    pub motion_offset: usize,
    pub motion_len: usize,
}

impl PageTable {
    pub fn new(motion_offset: usize, motion_len: usize, motions: usize) -> Result<Self, String> {
        if motion_len != motions * 2 * VERTEX_BYTES {
            return Err("Motion section length does not match the motion count".into());
        }
        Ok(Self {
            motions,
            page_motions: PAGE_MOTIONS,
            motion_offset,
            motion_len,
        })
    }
    pub fn page_count(&self) -> usize {
        self.motions.div_ceil(self.page_motions)
    }
    pub fn page_of(&self, motion: usize) -> usize {
        motion.min(self.motions) / self.page_motions
    }
    pub fn motions_in(&self, page: usize) -> Range<usize> {
        let start = page * self.page_motions;
        start..(start + self.page_motions).min(self.motions)
    }
    /// Byte range inside the whole payload for one page.
    pub fn bytes_of(&self, page: usize) -> Range<usize> {
        let motions = self.motions_in(page);
        let start = self.motion_offset + motions.start * 2 * VERTEX_BYTES;
        start..start + (motions.end - motions.start) * 2 * VERTEX_BYTES
    }
    /// Byte offset a GPU buffer must place this page at, so one page always
    /// lands on the same slot after eviction and re-upload.
    pub fn slot_of(&self, page: usize) -> usize {
        page * PAGE_BYTES
    }
    pub fn buffer_bytes(&self) -> usize {
        self.page_count() * PAGE_BYTES
    }
}

/// FNV-1a over a page. Used to skip GPU copies whose bytes did not change.
pub fn page_hash(bytes: &[u8]) -> u64 {
    let mut hash = 0xcbf2_9ce4_8422_2325_u64;
    for byte in bytes {
        hash ^= *byte as u64;
        hash = hash.wrapping_mul(0x0000_0100_0000_01b3);
    }
    hash
}

/// Worker result file: `u32 metadata length | metadata JSON | payload`.
pub fn frame_message(metadata: &[u8], payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(4 + metadata.len() + payload.len());
    out.extend_from_slice(&(metadata.len() as u32).to_le_bytes());
    out.extend_from_slice(metadata);
    out.extend_from_slice(payload);
    out
}

pub fn parse_message(bytes: Vec<u8>) -> Result<(Vec<u8>, Vec<u8>), String> {
    if bytes.len() < 4 {
        return Err("Worker result is truncated".into());
    }
    let head = u32::from_le_bytes(bytes[0..4].try_into().unwrap()) as usize;
    if 4 + head > bytes.len() {
        return Err("Worker result metadata length is invalid".into());
    }
    let metadata = bytes[4..4 + head].to_vec();
    let payload = bytes[4 + head..].to_vec();
    Ok((metadata, payload))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn payload_round_trips_with_aligned_sections() {
        let mut builder = Builder::new();
        builder.push(SECTION_CONTOUR, vec![1u8; 100]);
        // Two full pages plus a three-motion tail.
        builder.push(
            SECTION_MOTIONS,
            vec![2u8; PAGE_BYTES * 2 + 3 * 2 * VERTEX_BYTES],
        );
        builder.push(SECTION_STOCK, vec![3u8; 7]);
        let bytes = builder.finish();
        assert_eq!(builder_copy(&bytes), bytes);
        let payload = Payload::parse(bytes.clone()).unwrap();
        assert_eq!(payload.sections().len(), 3);
        assert_eq!(payload.sections()[0].offset % ALIGN, 0);
        assert_eq!(payload.slice(payload.find(SECTION_STOCK, 0).unwrap())[0], 3);
        let table = PageTable::new(
            payload.find(SECTION_MOTIONS, 0).unwrap().offset,
            payload.find(SECTION_MOTIONS, 0).unwrap().len,
            PAGE_MOTIONS * 2 + 3,
        )
        .unwrap();
        assert_eq!(table.page_count(), 3);
        assert_eq!(table.bytes_of(0).len(), PAGE_BYTES);
        assert_eq!(table.bytes_of(2).len(), 3 * 2 * VERTEX_BYTES);
        assert_eq!(table.bytes_of(1).start, table.motion_offset + PAGE_BYTES);
        assert_eq!(table.slot_of(2), PAGE_BYTES * 2);
    }

    fn builder_copy(bytes: &[u8]) -> Vec<u8> {
        // Rebuilding from the same sections must be byte-identical.
        let payload = Payload::parse(bytes.to_vec()).unwrap();
        let mut builder = Builder::new();
        for section in payload.sections() {
            builder.push(section.kind, payload.slice(section).to_vec());
        }
        builder.finish()
    }

    #[test]
    fn truncated_and_misaligned_payloads_are_rejected() {
        assert!(Payload::parse(vec![0; 4]).is_err());
        assert!(Payload::parse(b"NOTATRUCK".to_vec()).is_err());
        let mut builder = Builder::new();
        builder.push(SECTION_CONTOUR, vec![0; 32]);
        let mut bytes = builder.finish();
        bytes[12 + 8] = 1; // misalign the first section
        assert!(Payload::parse(bytes).is_err());
    }

    #[test]
    fn frame_messages_round_trip() {
        let message = frame_message(br#"{"a":1}"#, b"payload");
        let (metadata, payload) = parse_message(message).unwrap();
        assert_eq!(metadata, br#"{"a":1}"#);
        assert_eq!(payload, b"payload");
        assert!(parse_message(vec![1, 2]).is_err());
    }

    #[test]
    fn page_hash_changes_with_one_byte() {
        let a = vec![0u8; PAGE_BYTES];
        let mut b = a.clone();
        b[PAGE_BYTES - 1] = 1;
        assert_ne!(page_hash(&a), page_hash(&b));
        assert_eq!(page_hash(&a), page_hash(&a.clone()));
    }
}
