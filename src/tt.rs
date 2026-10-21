

use std::sync::atomic::{AtomicU8, AtomicU64, Ordering};
use crate::moves::Move;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
#[repr(u8)]
pub enum Bound {
    Exact      = 0,
    Lower = 1,
    Upper = 2,
}

const MATE_BAND: i32 = crate::syzygy::TB_WIN_SCORE - 1000;

#[derive(Copy, Clone, Debug)]
pub struct TtEntry {
    pub mv:    Move,
    pub score: i32,

    pub eval:  i32,
    pub depth: u8,
    pub bound: Bound,
}

const GEN_MASK: u8 = 0x7;
const GEN_SHIFT: u32 = 61;

pub const EVAL_NONE: i32 = -(1 << 14);

const EVAL_MAX: i32 = (1 << 14) - 1;

#[inline]
fn pack(mv: Move, score: i32, eval: i32, depth: u8, bound: Bound, generation: u8) -> u64 {
    let eval = if eval == EVAL_NONE { EVAL_NONE } else { eval.clamp(-EVAL_MAX, EVAL_MAX) };
    let s = (score as u64) & 0x1F_FFFF;
    let e = (eval as u64) & 0x7FFF;
    let d = (depth.min(127) as u64) & 0x7F;
    let b = (bound as u64) & 0x3;
    let m = (mv.0 & 0xFFFF) as u64;
    let g = (generation & GEN_MASK) as u64;
    s | (e << 21) | (d << 36) | (b << 43) | (m << 45) | (g << GEN_SHIFT)
}

#[inline]
fn sext(v: u64, bits: u32) -> i32 {
    ((v << (64 - bits)) as i64 >> (64 - bits)) as i32
}

#[inline]
fn unpack(data: u64) -> (Move, i32, i32, u8, Bound, u8) {
    let score = sext(data, 21);
    let eval  = sext(data >> 21, 15);
    let depth = ((data >> 36) & 0x7F) as u8;
    let bound = match (data >> 43) & 0x3 {
        1 => Bound::Lower,
        2 => Bound::Upper,
        _ => Bound::Exact,
    };
    let mv         = Move(((data >> 45) & 0xFFFF) as u32);
    let generation = (data >> GEN_SHIFT) as u8 & GEN_MASK;
    (mv, score, eval, depth, bound, generation)
}

pub struct TranspositionTable {

    table:      Vec<AtomicU64>,
    mask:       usize,

    generation: AtomicU8,
}

impl TranspositionTable {
    pub fn new(mb: usize) -> Self {
        let bytes   = mb * 1024 * 1024;
        let entries = (bytes / 16).next_power_of_two() >> 1;
        let entries = entries.max(1);
        let mut table = Vec::with_capacity(entries * 2);
        for _ in 0..entries * 2 { table.push(AtomicU64::new(0)); }
        TranspositionTable { table, mask: entries - 1, generation: AtomicU8::new(0) }
    }

    #[inline]
    fn index(&self, key: u64) -> usize { ((key as usize) & self.mask) * 2 }

    pub fn new_generation(&self) {
        self.generation.fetch_add(1, Ordering::Relaxed);
    }

    #[inline]
    pub fn probe(&self, key: u64) -> Option<TtEntry> {
        let i    = self.index(key);
        let data = self.table[i + 1].load(Ordering::Relaxed);
        let k    = self.table[i    ].load(Ordering::Relaxed);
        if k ^ data != key { return None; }
        let (mv, score, eval, depth, bound, _gen) = unpack(data);
        Some(TtEntry { mv, score, eval, depth, bound })
    }

    #[inline]
    pub fn store(&self, key: u64, mv: Move, score: i32, eval: i32, depth: u8, bound: Bound) {
        let depth    = depth.min(127);
        let i        = self.index(key);
        let old_data = self.table[i + 1].load(Ordering::Relaxed);
        let old_k    = self.table[i    ].load(Ordering::Relaxed);
        let old_key  = old_k ^ old_data;
        let cur_gen  = self.generation.load(Ordering::Relaxed) & GEN_MASK;

        let (_, _, _, old_depth, _, old_gen) = unpack(old_data);

        let replace = old_data == 0
            || if old_key == key {
                depth >= old_depth
            } else {
                old_gen != cur_gen || depth >= old_depth
            };

        if replace {
            let data = pack(mv, score, eval, depth, bound, cur_gen);

            self.table[i + 1].store(data,       Ordering::Relaxed);
            self.table[i    ].store(key ^ data, Ordering::Relaxed);
        }
    }

    #[inline]
    pub fn score_to_tt(score: i32, ply: usize) -> i32 {
        if score > MATE_BAND { score + ply as i32 }
        else if score < -MATE_BAND { score - ply as i32 }
        else { score }
    }

    #[inline]
    pub fn score_from_tt(score: i32, ply: usize) -> i32 {
        if score > MATE_BAND { score - ply as i32 }
        else if score < -MATE_BAND { score + ply as i32 }
        else { score }
    }

    pub fn clear(&self) {
        for slot in &self.table { slot.store(0, Ordering::Relaxed); }
        self.generation.store(0, Ordering::Relaxed);
    }

    pub fn hashfull(&self) -> usize {
        let entries = self.table.len() / 2;
        let sample  = entries.min(1000);
        let cur_gen = self.generation.load(Ordering::Relaxed) & GEN_MASK;
        let used = (0..sample)
            .filter(|&i| {
                let data = self.table[i * 2 + 1].load(Ordering::Relaxed);
                data != 0 && (data >> GEN_SHIFT) as u8 & GEN_MASK == cur_gen
            })
            .count();
        used * 1000 / sample.max(1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::eval::{INF, MATE_SCORE};

    #[test]
    fn pack_round_trips() {
        let mv = Move(0xFFFF);
        for &score in &[0, 1, -1, 12_345, -12_345, MATE_SCORE - 5, -(MATE_SCORE - 5), 800_000 - 300, INF, -INF] {
            for &eval in &[0, 37, -37, EVAL_MAX, -EVAL_MAX, EVAL_NONE] {
                for &bound in &[Bound::Exact, Bound::Lower, Bound::Upper] {
                    let (m, s, e, d, b, g) = unpack(pack(mv, score, eval, 127, bound, 7));
                    assert_eq!((m, s, e, d, b, g), (mv, score, eval, 127, bound, 7));
                }
            }
        }

        assert_eq!(unpack(pack(mv, 0, 40_000, 1, Bound::Exact, 0)).2, EVAL_MAX);
        assert_eq!(unpack(pack(mv, 0, -40_000, 1, Bound::Exact, 0)).2, -EVAL_MAX);
    }
}
