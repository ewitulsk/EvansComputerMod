//! The cell site's uplink scheduler, run in batches (every
//! [`crate::site::SCHED_PERIOD_MS`], about 10 Hz): each phone gets a grant
//! for the next windows, so the per-frame timing is the radio's job.
//!
//! Uplink is split by sub-channel (FDMA, the framed PHY's quarters) and then
//! by time: a phone alone on its sub-channel may transmit in every slot; `m`
//! phones sharing one take turns in runs of `run` slots. One sub-channel
//! (the system information's `rach_sub`) is kept for random access.
//!
//! Neighbour coordination: sub-channels that same-channel neighbour cells
//! use (their system information's `ul_mask`, as phones report it in
//! measurement reports) are taken last, and the starting point rotates with
//! the cell id, so two cells at an edge settle on different quarters.
//!
//! Voice first (semi-persistent scheduling for a call's voice bearer): phones
//! with a voice bearer are placed before the others, each on a data sub-channel
//! of its own while there are enough, so a call's 20 ms frames always find a
//! slot; best-effort phones share what is left.

use alloc::vec::Vec;

use crate::air::Grant;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SchedUe {
    pub addr: u32,
    /// Bytes waiting (buffer status report).
    pub bsr: u32,
    /// The CQI the site measures on this phone's uplink.
    pub cqi: u8,
    /// The phone has a voice bearer (a call).
    pub voice: bool,
}

/// Data sub-channels in preference order.
pub fn sub_preference(cell_id: u32, rach_sub: u8, neighbour_mask: u8) -> Vec<u8> {
    let mut subs: Vec<u8> = (1..=4u8).filter(|s| *s != rach_sub).collect();
    let n = subs.len() as u32;
    subs.sort_by_key(|s| {
        let busy = neighbour_mask & (1 << (s - 1)) != 0;
        (busy, ((*s as u32) + n - cell_id % n) % n)
    });
    subs
}

/// Grants for `ues` valid in `[from, to)`; returns them and the mask of
/// sub-channels used.
pub fn plan(ues: &[SchedUe], subs: &[u8], from: u64, to: u64, run: u16) -> (Vec<(u32, Grant)>, u8) {
    // A stable order: the same phones get the same resources batch after
    // batch, so a grant arriving a little late never overlaps a neighbour's.
    let mut order: Vec<SchedUe> = ues.to_vec();
    order.sort_by_key(|u| (!u.voice, u.addr));
    let k = subs.len().max(1);
    let mut per_sub: Vec<Vec<SchedUe>> = alloc::vec![Vec::new(); k];
    let voice = order.iter().filter(|u| u.voice).count();
    // voice phones on sub-channels of their own; the rest share the others
    // (all of them when calls fill every sub-channel)
    let rest_from = if voice < k { voice } else { 0 };
    let mut j = 0;
    for (i, u) in order.iter().enumerate() {
        if u.voice {
            per_sub[i % k].push(*u);
        } else {
            per_sub[rest_from + j % (k - rest_from)].push(*u);
            j += 1;
        }
    }
    let mut out = Vec::new();
    let mut mask = 0u8;
    for (si, members) in per_sub.iter().enumerate() {
        if members.is_empty() {
            continue;
        }
        let sub = subs.get(si).copied().unwrap_or(1);
        mask |= 1 << (sub - 1);
        let m = members.len() as u16;
        for (j, u) in members.iter().enumerate() {
            let g = if m == 1 {
                Grant { from_slot: from, to_slot: to, period: 1, offset: 0, len: 1, subchannel: sub, cqi: u.cqi.max(1) }
            } else {
                let run = run.max(1);
                Grant { from_slot: from, to_slot: to, period: run * m, offset: run * j as u16, len: run, subchannel: sub, cqi: u.cqi.max(1) }
            };
            out.push((u.addr, g));
        }
    }
    (out, mask)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn neighbours_steer_sub_channels() {
        let a = sub_preference(1, 4, 0);
        let b = sub_preference(2, 4, 0);
        assert_eq!(a.len(), 3);
        assert_ne!(a[0], b[0], "rotation by cell id");
        // a neighbour using sub-channel a[0] pushes it last
        let c = sub_preference(1, 4, 1 << (a[0] - 1));
        assert_eq!(*c.last().unwrap(), a[0]);
        assert!(!a.contains(&4), "random access sub-channel never granted");
    }

    #[test]
    fn grants_never_overlap_within_a_cell() {
        let ues: Vec<SchedUe> = (0..7).map(|i| SchedUe { addr: 100 + i, bsr: if i % 2 == 0 { 1000 } else { 0 }, cqi: 7, voice: false }).collect();
        let subs = sub_preference(3, 4, 0);
        let (g, mask) = plan(&ues, &subs, 1000, 1300, 4);
        assert_eq!(g.len(), 7);
        assert_eq!(mask.count_ones(), 3);
        for slot in 1000..1300 {
            for sub in 1..=4u8 {
                let users = g.iter().filter(|(_, x)| x.subchannel == sub && x.allows(slot)).count();
                assert!(users <= 1, "slot {} sub {}: {} phones", slot, sub, users);
            }
        }
        // every phone gets airtime
        for (a, x) in &g {
            assert!((1000..1300).any(|s| x.allows(s)), "{} starved", a);
        }
        // one phone alone: continuous grant
        let (g, _) = plan(&ues[..1], &subs, 0, 10, 4);
        assert_eq!((g[0].1.period, g[0].1.len), (1, 1));
    }

    #[test]
    fn a_call_gets_a_sub_channel_of_its_own() {
        let mut ues: Vec<SchedUe> = (0..6).map(|i| SchedUe { addr: 100 + i, bsr: 0, cqi: 7, voice: false }).collect();
        ues[4].voice = true;
        let subs = sub_preference(1, 4, 0);
        let (g, _) = plan(&ues, &subs, 0, 400, 4);
        let call = g.iter().find(|(a, _)| *a == 104).unwrap().1;
        assert_eq!((call.period, call.len), (1, 1), "every slot");
        assert!(g.iter().filter(|(a, _)| *a != 104).all(|(_, x)| x.subchannel != call.subchannel), "nobody shares it");
        for slot in 0..400 {
            for sub in 1..=4u8 {
                assert!(g.iter().filter(|(_, x)| x.subchannel == sub && x.allows(slot)).count() <= 1);
            }
        }
        // more calls than sub-channels: everyone shares, nobody starves
        for u in ues.iter_mut() {
            u.voice = true;
        }
        let (g, _) = plan(&ues, &subs, 0, 400, 4);
        for (a, x) in &g {
            assert!((0..20).any(|s| x.allows(s)), "{} gets a slot every 20 ms", a);
        }
    }
}
