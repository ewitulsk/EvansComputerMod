//! IP packets over the air: segmentation into Data items, reassembly, and
//! downlink duplicate suppression by tunnel sequence number.

use alloc::collections::BTreeMap;
use alloc::vec::Vec;

use crate::air::{Item, DATA_OVERHEAD};
use crate::crypto::ReplayWindow;

/// Split `pkt` into Data items whose encoded size is at most `max_item` bytes.
pub fn segment(seq: u32, pkt: &[u8], max_item: usize) -> Vec<Item> {
    let room = max_item.saturating_sub(DATA_OVERHEAD).max(16);
    let count = pkt.len().div_ceil(room).max(1);
    if count > 255 {
        return Vec::new();
    }
    let per = pkt.len().div_ceil(count).max(1);
    let mut v = Vec::with_capacity(count);
    for (i, ch) in pkt.chunks(per).enumerate() {
        v.push(Item::Data { seq, idx: i as u8, count: count as u8, bytes: ch.to_vec() });
    }
    if v.is_empty() {
        v.push(Item::Data { seq, idx: 0, count: 1, bytes: Vec::new() });
    }
    v
}

struct Partial {
    parts: Vec<Option<Vec<u8>>>,
    have: usize,
    started_ms: u64,
}

/// Reassembles segments (any order); incomplete packets expire.
#[derive(Default)]
pub struct Reassembly {
    partial: BTreeMap<u32, Partial>,
}

/// Most packets being reassembled at once (per peer).
const MAX_PARTIAL: usize = 32;
const EXPIRE_MS: u64 = 2000;
/// Largest reassembled packet.
pub const MAX_PACKET: usize = 1500;

impl Reassembly {
    pub fn push(&mut self, seq: u32, idx: u8, count: u8, bytes: &[u8], now_ms: u64) -> Option<Vec<u8>> {
        if count == 1 {
            return (bytes.len() <= MAX_PACKET).then(|| bytes.to_vec());
        }
        self.partial.retain(|_, p| now_ms.saturating_sub(p.started_ms) < EXPIRE_MS);
        if !self.partial.contains_key(&seq) && self.partial.len() >= MAX_PARTIAL {
            let oldest = *self.partial.iter().min_by_key(|(_, p)| p.started_ms).map(|(k, _)| k)?;
            self.partial.remove(&oldest);
        }
        let p = self.partial.entry(seq).or_insert_with(|| Partial { parts: alloc::vec![None; count as usize], have: 0, started_ms: now_ms });
        if p.parts.len() != count as usize || idx as usize >= p.parts.len() {
            self.partial.remove(&seq);
            return None;
        }
        if p.parts[idx as usize].is_none() {
            p.parts[idx as usize] = Some(bytes.to_vec());
            p.have += 1;
        }
        if p.have < p.parts.len() {
            return None;
        }
        let p = self.partial.remove(&seq)?;
        let mut out = Vec::new();
        for b in p.parts.into_iter().flatten() {
            out.extend_from_slice(&b);
        }
        (out.len() <= MAX_PACKET).then_some(out)
    }
}

/// Drops downlink packets already delivered (make-before-break handover
/// delivers some through both sites).
#[derive(Default)]
pub struct Dedup {
    w: ReplayWindow,
}

impl Dedup {
    /// True the first time `seq` is seen.
    pub fn fresh(&mut self, seq: u32) -> bool {
        // Tunnel seqs start at 1 per session; 0 is never sent.
        if !self.w.check(seq as u64) {
            return false;
        }
        self.w.accept(seq as u64);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn segment_reassemble_any_order() {
        let pkt: Vec<u8> = (0..1500u32).map(|i| (i * 7) as u8).collect();
        let segs = segment(9, &pkt, 400);
        assert!(segs.len() >= 4);
        for s in &segs {
            assert!(s.encoded_len() <= 400);
        }
        let mut r = Reassembly::default();
        let mut out = None;
        for s in segs.iter().rev() {
            if let Item::Data { seq, idx, count, bytes } = s {
                out = r.push(*seq, *idx, *count, bytes, 0).or(out);
            }
        }
        assert_eq!(out, Some(pkt));
        // single segment
        let one = segment(1, &[1, 2, 3], 1400);
        assert_eq!(one.len(), 1);
    }

    #[test]
    fn partial_expires_and_dup_parts_are_ignored() {
        let mut r = Reassembly::default();
        assert_eq!(r.push(1, 0, 2, &[1], 0), None);
        assert_eq!(r.push(1, 0, 2, &[1], 10), None);
        assert_eq!(r.push(1, 1, 2, &[2], 20), Some(alloc::vec![1, 2]));
        assert_eq!(r.push(2, 0, 2, &[1], 0), None);
        assert_eq!(r.push(2, 1, 2, &[2], 5000), None, "first half expired");
        assert_eq!(r.push(3, 0, 3, &[1], 0), None);
        assert_eq!(r.push(3, 1, 2, &[2], 0), None, "count changed: dropped");
    }

    #[test]
    fn dedup() {
        let mut d = Dedup::default();
        assert!(d.fresh(1) && d.fresh(3) && d.fresh(2));
        assert!(!d.fresh(2) && !d.fresh(1));
    }
}
