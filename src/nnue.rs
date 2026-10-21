

use std::cell::Cell;
use std::fs;
use std::sync::{Arc, RwLock, RwLockReadGuard};

use crate::board::{Board, Color, Piece};

pub const HL: usize = 1024;

const INPUT_SIZE: usize = 768;

const INPUT_BUCKETS: usize = 8;

#[rustfmt::skip]
const KING_BUCKETS: [u8; 64] = [
    0, 1, 2, 3, 3, 2, 1, 0,
    4, 4, 5, 5, 5, 5, 4, 4,
    6, 6, 6, 6, 6, 6, 6, 6,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
    7, 7, 7, 7, 7, 7, 7, 7,
];

const OUTPUT_BUCKETS: usize = 8;

const L2: usize = 16;

const L3: usize = 32;

const QA: i64 = 255;
const QB: i64 = 64;
const SCALE: i64 = 400;

const L1_SCALE: f64 = 2.0 / (QA * QA * QB) as f64;

const EMBEDDED_NET: &[u8] = include_bytes!("../nets/net.nnue");

pub struct Net {

    l0w: AlignedBuf<i16>,

    l0b: AlignedBuf<i16>,

    l1w_pairs: AlignedBuf<i16>,

    l1b: AlignedBuf<f32>,

    l2w_t: AlignedBuf<f32>,

    l2b: AlignedBuf<f32>,

    l3w: AlignedBuf<f32>,

    l3b: AlignedBuf<f32>,
}

static NET: RwLock<Option<Arc<Net>>> = RwLock::new(None);

pub fn load_embedded() {
    let net = parse_net(EMBEDDED_NET)
        .unwrap_or_else(|e| panic!("embedded NNUE net failed to parse: {e}"));
    *NET.write().unwrap() = Some(Arc::new(net));
}

pub fn load(path: &str) -> Result<(), String> {
    let buf = fs::read(path).map_err(|e| format!("reading '{path}': {e}"))?;
    let net = parse_net(&buf)?;
    *NET.write().unwrap() = Some(Arc::new(net));
    Ok(())
}

pub fn current_net() -> Option<Arc<Net>> {
    NET.read().unwrap().clone()
}

pub fn read_lock() -> RwLockReadGuard<'static, Option<Arc<Net>>> {
    NET.read().unwrap()
}

// ================= Loading =================

/// A 64-byte aligned, fixed-length buffer of plain numbers. Weight rows then
/// start on a cache line, so no SIMD load splits one; a plain `Vec<i16>` is
/// only 16-byte aligned for large allocations, which cost ~10% of eval speed.
pub struct AlignedBuf<T: Copy> {
    buf: Vec<Align64>,
    len: usize,
    _t: std::marker::PhantomData<T>,
}

#[repr(C, align(64))]
#[derive(Clone, Copy)]
struct Align64([u8; 64]);

impl<T: Copy> From<Vec<T>> for AlignedBuf<T> {
    fn from(v: Vec<T>) -> Self {
        let bytes = std::mem::size_of_val(v.as_slice());
        let mut buf = vec![Align64([0; 64]); bytes.div_ceil(64)];
        // SAFETY: buf holds at least `bytes` bytes; T is plain data (i16 / f32).
        unsafe { std::ptr::copy_nonoverlapping(v.as_ptr() as *const u8, buf.as_mut_ptr() as *mut u8, bytes) };
        AlignedBuf { buf, len: v.len(), _t: std::marker::PhantomData }
    }
}

impl<T: Copy> std::ops::Deref for AlignedBuf<T> {
    type Target = [T];
    fn deref(&self) -> &[T] {
        // SAFETY: the buffer holds `len` initialised T values, 64-byte aligned.
        unsafe { std::slice::from_raw_parts(self.buf.as_ptr() as *const T, self.len) }
    }
}

fn read_i16_block(buf: &[u8], offset: &mut usize, n: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        out.push(i16::from_le_bytes([buf[*offset], buf[*offset + 1]]));
        *offset += 2;
    }
    out
}

fn read_f32_block(buf: &[u8], offset: &mut usize, n: usize) -> Vec<f32> {
    let mut out = Vec::with_capacity(n);
    for _ in 0..n {
        let b = &buf[*offset..*offset + 4];
        out.push(f32::from_le_bytes([b[0], b[1], b[2], b[3]]));
        *offset += 4;
    }
    out
}

/// Parse a `bullet` export of the midlevel net, written by
/// erebus-trainer-midlevel's `save_format`. Little-endian, in this order

fn parse_net(buf: &[u8]) -> Result<Net, String> {
    let n_l0w = INPUT_BUCKETS * INPUT_SIZE * HL;
    let n_l0b = HL;
    let n_l1w = OUTPUT_BUCKETS * L2 * 2 * HL;
    let n_f32 = OUTPUT_BUCKETS * (L2 + L3 * L2 + L3 + L3 + 1);
    let expected = (n_l0w + n_l0b + n_l1w) * 2 + n_f32 * 4;

    if buf.len() < expected {
        return Err(format!(
            "net data is {} bytes, expected at least {expected} for a \
             (768x{INPUT_BUCKETS} -> {HL}) x 2 -> {L2} -> {L3} -> 1 net with {OUTPUT_BUCKETS} \
             output buckets. Either a size constant is wrong for this file, or this is not an \
             erebus-trainer-midlevel export (a mini net is ~1.5 MB).",
            buf.len()
        ));
    }

    let mut off = 0usize;
    let l0w = read_i16_block(buf, &mut off, n_l0w);
    let l0b = read_i16_block(buf, &mut off, n_l0b);
    let l1w = read_i16_block(buf, &mut off, n_l1w);
    let l1b = read_f32_block(buf, &mut off, OUTPUT_BUCKETS * L2);
    let l2w = read_f32_block(buf, &mut off, OUTPUT_BUCKETS * L3 * L2);
    let l2b = read_f32_block(buf, &mut off, OUTPUT_BUCKETS * L3);
    let l3w = read_f32_block(buf, &mut off, OUTPUT_BUCKETS * L3);
    let l3b = read_f32_block(buf, &mut off, OUTPUT_BUCKETS);

    let max_l1w = l1w.iter().map(|&x| (x as i32).abs()).max().unwrap_or(0);
    if QA as i32 * max_l1w > i16::MAX as i32 {
        return Err(format!(
            "l1w |max| = {max_l1w}: QA({QA}) * {max_l1w} = {} exceeds i16 range; the 2*HL -> L2 \
             layer's i32 sums are sized for |l1w| <= {}. Retrain with tighter l1w clipping.",
            QA as i32 * max_l1w,
            i16::MAX as i32 / QA as i32,
        ));
    }

    let floats = [&l1b, &l2w, &l2b, &l3w, &l3b];
    if floats.iter().any(|v| v.iter().any(|x| !x.is_finite())) {
        return Err("the float layers (l1b, l2w, l2b, l3w, l3b) contain NaN or infinity -- \
                    the net is corrupt or diverged in training"
            .to_string());
    }

    let nz = l0w.iter().filter(|&&x| x != 0).count();
    let nz_frac = nz as f64 / l0w.len().max(1) as f64;
    if nz_frac < 0.01 {
        return Err(format!(
            "feature-transformer weights are {:.3}% non-zero ({nz}/{} values) -- this is not a \
             trained net. Every position would evaluate to the same constant.",
            nz_frac * 100.0,
            l0w.len(),
        ));
    }

    let mut l1w_pairs = vec![0i16; l1w.len()];
    for b in 0..OUTPUT_BUCKETS {
        for o in 0..L2 {
            for i in 0..2 * HL {
                let (p, k) = (i / 2, i % 2);
                l1w_pairs[((b * HL + p) * L2 + o) * 2 + k] = l1w[(b * L2 + o) * 2 * HL + i];
            }
        }
    }

    let mut l2w_t = vec![0f32; l2w.len()];
    for b in 0..OUTPUT_BUCKETS {
        for j in 0..L3 {
            for o in 0..L2 {
                l2w_t[(b * L2 + o) * L3 + j] = l2w[(b * L3 + j) * L2 + o];
            }
        }
    }

    Ok(Net {
        l0w: l0w.into(),
        l0b: l0b.into(),
        l1w_pairs: l1w_pairs.into(),
        l1b: l1b.into(),
        l2w_t: l2w_t.into(),
        l2b: l2b.into(),
        l3w: l3w.into(),
        l3b: l3b.into(),
    })
}

#[inline(always)]
pub fn king_bucket(perspective: Color, king_sq: u8) -> u8 {
    let rel = match perspective {
        Color::White => king_sq,
        Color::Black => king_sq ^ 56,
    };
    KING_BUCKETS[rel as usize]
}

#[inline(always)]
pub fn output_bucket(piece_count: u32) -> usize {
    const DIV: u32 = 32u32.div_ceil(OUTPUT_BUCKETS as u32);
    ((piece_count.saturating_sub(2) / DIV) as usize).min(OUTPUT_BUCKETS - 1)
}

#[inline(always)]
fn feature_index(perspective: Color, bucket: u8, piece: Piece, piece_color: Color, sq: u8) -> usize {
    feature_index_raw(perspective, bucket, piece as usize, piece_color as usize, sq)
}

#[inline(always)]
fn feature_index_raw(perspective: Color, bucket: u8, piece: usize, piece_color: usize, sq: u8) -> usize {
    let (rel_color, rel_sq) = match perspective {
        Color::White => (piece_color, sq as usize),
        Color::Black => (piece_color ^ 1, (sq ^ 56) as usize),
    };
    INPUT_SIZE * bucket as usize + 384 * rel_color + 64 * piece + rel_sq
}

#[inline(always)]
fn weight_row(net: &Net, feature: usize) -> &[i16] {
    let base = feature * HL;
    &net.l0w[base..base + HL]
}

#[derive(Clone, PartialEq, Eq, Debug)]
#[repr(C, align(64))]
pub struct Accumulator {
    pub v: [i16; HL],

    pub bucket: u8,
}

impl Accumulator {
    pub fn zeroed() -> Self {
        Accumulator { v: [0; HL], bucket: 0 }
    }
}

#[derive(Clone, PartialEq, Eq, Debug)]
pub struct DualAccumulator {
    pub white: Accumulator,
    pub black: Accumulator,
}

impl DualAccumulator {
    pub fn zeroed() -> Self {
        DualAccumulator { white: Accumulator::zeroed(), black: Accumulator::zeroed() }
    }

    #[inline(always)]
    pub fn get(&self, perspective: Color) -> &Accumulator {
        match perspective {
            Color::White => &self.white,
            Color::Black => &self.black,
        }
    }

    #[inline(always)]
    pub fn get_mut(&mut self, perspective: Color) -> &mut Accumulator {
        match perspective {
            Color::White => &mut self.white,
            Color::Black => &mut self.black,
        }
    }
}

pub type FeatDelta = (Piece, Color, u8, bool);

#[derive(Clone)]
struct Pending {
    feats:   [FeatDelta; 4],
    n:       u8,

    refresh: u8,

    buckets: [u8; 2],

    pieces:  [[u64; 6]; 2],
}

impl Pending {
    fn empty() -> Self {
        Pending {
            feats:   [(Piece::Pawn, Color::White, 0, false); 4],
            n:       0,
            refresh: 0,
            buckets: [0; 2],
            pieces:  [[0; 6]; 2],
        }
    }
}

#[derive(Clone)]
pub struct AccStack {
    accs: Vec<DualAccumulator>,

    meta: Vec<Pending>,
    top: usize,

    clean: usize,
}

impl AccStack {
    pub fn new() -> Self {
        AccStack { accs: vec![DualAccumulator::zeroed()], meta: vec![Pending::empty()], top: 0, clean: 0 }
    }

    pub fn reset(&mut self, net: &Net, sq_piece: &[Option<(Piece, Color)>; 64]) {
        self.top = 0;
        self.clean = 0;
        refresh_all(net, sq_piece, &mut self.accs[0]);
        self.meta[0].buckets = [self.accs[0].white.bucket, self.accs[0].black.bucket];
    }

    #[inline(always)]
    pub fn bucket(&self, perspective: Color) -> u8 {
        self.meta[self.top].buckets[perspective as usize]
    }

    #[inline]
    pub fn push(&mut self, feats: &[FeatDelta], refresh: u8, buckets: [u8; 2], pieces: &[[u64; 6]; 2]) {
        if self.top + 1 == self.accs.len() {
            self.accs.push(DualAccumulator::zeroed());
            self.meta.push(Pending::empty());
        }
        self.top += 1;
        let m = &mut self.meta[self.top];
        m.feats[..feats.len()].copy_from_slice(feats);
        m.n = feats.len() as u8;
        m.refresh = refresh;
        m.buckets = buckets;
        if refresh != 0 {
            m.pieces = *pieces;
        }
    }

    #[inline]
    pub fn pop(&mut self) -> bool {
        if self.top == 0 {
            return false;
        }
        self.top -= 1;
        self.clean = self.clean.min(self.top);
        true
    }

    #[inline]
    pub fn current(&mut self, net: &Net, cache: &mut BucketCache) -> &DualAccumulator {
        while self.clean < self.top {
            let i = self.clean + 1;
            let (done, pending) = self.accs.split_at_mut(i);
            let (parent, child) = (&done[i - 1], &mut pending[0]);
            let m = &self.meta[i];
            for persp in [Color::White, Color::Black] {
                let acc = child.get_mut(persp);
                if m.refresh & (1 << persp as u8) != 0 {
                    refresh_cached(net, cache, &m.pieces, persp, acc);
                } else {
                    update_from_parent(net, persp, parent.get(persp), acc, &m.feats[..m.n as usize]);
                }
            }
            self.clean = i;
        }
        &self.accs[self.top]
    }
}

const EVAL_CACHE_BITS: u32 = 16;

#[derive(Clone)]
pub struct EvalCache {
    slots: Box<[Cell<u64>]>,
}

impl EvalCache {
    pub fn new() -> Self {
        EvalCache { slots: (0..1usize << EVAL_CACHE_BITS).map(|_| Cell::new(0)).collect() }
    }

    pub fn clear(&mut self) {
        for s in self.slots.iter() {
            s.set(0);
        }
    }

    #[inline(always)]
    fn slot(&self, hash: u64) -> &Cell<u64> {
        &self.slots[hash as usize & ((1usize << EVAL_CACHE_BITS) - 1)]
    }

    #[inline(always)]
    fn probe(&self, hash: u64) -> Option<i32> {
        let e = self.slot(hash).get();
        (e >> 32 == hash >> 32 && e != 0).then_some(e as u32 as i32)
    }

    #[inline(always)]
    fn store(&self, hash: u64, eval: i32) {
        self.slot(hash).set((hash & 0xFFFF_FFFF_0000_0000) | eval as u32 as u64);
    }
}

const QA_I16: i16 = QA as i16;

#[cfg(target_arch = "x86_64")]
static NZ_LUT: [[u16; 8]; 256] = {
    let mut t = [[0u16; 8]; 256];
    let mut m = 0;
    while m < 256 {
        let (mut k, mut b) = (0, 0);
        while b < 8 {
            if (m >> b) & 1 == 1 {
                t[m][k] = b as u16;
                k += 1;
            }
            b += 1;
        }
        m += 1;
    }
    t
};

const FLUSH_PAIRS: usize = 128;
const _: () = assert!(
    FLUSH_PAIRS as i64 * 2 * ((QA * QA) >> 1) * 128 <= i32::MAX as i64,
    "l1_forward's i32 lanes could overflow",
);
const _: () = assert!(L2 == 16, "l1_forward_avx2 is written for 16 outputs");
const _: () = assert!(L3.is_multiple_of(8), "l2_l3_avx2 works in 8-wide f32 blocks");
const _: () = assert!(HL.is_multiple_of(64), "l1_forward_avx2 scans 64 values per step");

#[inline]
fn acc_add(acc: &mut [i16], row: &[i16]) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return acc_addsub_avx2(acc, row, false) }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    if std::is_x86_feature_detected!("avx2") {
        unsafe { return acc_addsub_avx2(acc, row, false) }
    }
    #[cfg(target_arch = "aarch64")]
    unsafe { return acc_addsub_neon(acc, row, false) }
    #[allow(unreachable_code)]
    acc_addsub_scalar(acc, row, false)
}

#[inline]
fn acc_sub(acc: &mut [i16], row: &[i16]) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return acc_addsub_avx2(acc, row, true) }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    if std::is_x86_feature_detected!("avx2") {
        unsafe { return acc_addsub_avx2(acc, row, true) }
    }
    #[cfg(target_arch = "aarch64")]
    unsafe { return acc_addsub_neon(acc, row, true) }
    #[allow(unreachable_code)]
    acc_addsub_scalar(acc, row, true)
}

#[inline]
fn acc_set(acc: &mut [i16], bias: &[i16]) {
    let n = acc.len().min(bias.len());
    acc[..n].copy_from_slice(&bias[..n]);
}

#[inline]
fn screlu_half(acc: &[i16], out: &mut [i16]) {
    for (o, &a) in out.iter_mut().zip(acc) {
        let c = a.clamp(0, QA_I16) as u16;
        *o = ((c * c) >> 1) as i16;
    }
}

#[inline]
fn l1_forward(us: &[i16; HL], them: &[i16; HL], w: &[i16], out: &mut [i64; L2]) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return l1_forward_avx2(us, them, w, out) }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    if std::is_x86_feature_detected!("avx2") {
        unsafe { return l1_forward_avx2(us, them, w, out) }
    }
    #[allow(unreachable_code)]
    {
        let mut c = [0i16; 2 * HL];
        screlu_half(us, &mut c[..HL]);
        screlu_half(them, &mut c[HL..]);
        l1_sparse_scalar(&c, w, out)
    }
}

fn l1_sparse_scalar(c: &[i16; 2 * HL], w: &[i16], out: &mut [i64; L2]) {
    *out = [0; L2];
    for p in 0..HL {
        let (a, b) = (c[2 * p], c[2 * p + 1]);
        if a == 0 && b == 0 {
            continue;
        }
        let row = &w[p * 2 * L2..(p + 1) * 2 * L2];
        for o in 0..L2 {
            out[o] += a as i64 * row[2 * o] as i64 + b as i64 * row[2 * o + 1] as i64;
        }
    }
}

fn acc_addsub_scalar(acc: &mut [i16], row: &[i16], sub: bool) {
    let n = acc.len().min(row.len());
    if sub {
        for h in 0..n { acc[h] = acc[h].wrapping_sub(row[h]); }
    } else {
        for h in 0..n { acc[h] = acc[h].wrapping_add(row[h]); }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn acc_addsub_avx2(acc: &mut [i16], row: &[i16], sub: bool) {
    use std::arch::x86_64::*;
    let n = acc.len().min(row.len());
    let mut h = 0;
    while h + 16 <= n {
        let a = _mm256_loadu_si256(acc.as_ptr().add(h) as *const __m256i);
        let r = _mm256_loadu_si256(row.as_ptr().add(h) as *const __m256i);
        let s = if sub { _mm256_sub_epi16(a, r) } else { _mm256_add_epi16(a, r) };
        _mm256_storeu_si256(acc.as_mut_ptr().add(h) as *mut __m256i, s);
        h += 16;
    }
    if sub {
        while h < n { acc[h] = acc[h].wrapping_sub(row[h]); h += 1; }
    } else {
        while h < n { acc[h] = acc[h].wrapping_add(row[h]); h += 1; }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn l1_forward_avx2(us: &[i16; HL], them: &[i16; HL], w: &[i16], out: &mut [i64; L2]) {
    use std::arch::x86_64::*;
    use std::mem::MaybeUninit;
    debug_assert!(w.len() >= HL * 2 * L2);
    let zero = _mm256_setzero_si256();
    let qa = _mm256_set1_epi16(QA_I16);

    #[repr(C, align(64))]
    struct Sq([i16; 2 * HL]);

    let mut sq = MaybeUninit::<Sq>::uninit();
    let sqp = sq.as_mut_ptr() as *mut i16;
    let mut list = MaybeUninit::<[u16; HL + 8]>::uninit();
    let lp = list.as_mut_ptr() as *mut u16;

    let mut count = 0usize;
    for (half, src) in [us, them].into_iter().enumerate() {
        let sp = src.as_ptr();
        for step in 0..HL / 64 {
            let mut m = [0usize; 4];
            for (b, mb) in m.iter_mut().enumerate() {
                let i = step * 64 + b * 16;
                let c = _mm256_min_epi16(_mm256_max_epi16(_mm256_loadu_si256(sp.add(i) as *const __m256i), zero), qa);

                let v = _mm256_srli_epi16::<1>(_mm256_mullo_epi16(c, c));
                _mm256_store_si256(sqp.add(half * HL + i) as *mut __m256i, v);

                *mb = _mm256_movemask_ps(_mm256_castsi256_ps(_mm256_cmpgt_epi32(v, zero))) as usize;
            }
            for (b, &mb) in m.iter().enumerate() {
                let idx = _mm_add_epi16(
                    _mm_loadu_si128(NZ_LUT[mb].as_ptr() as *const __m128i),
                    _mm_set1_epi16(((half * HL + step * 64 + b * 16) / 2) as i16),
                );
                _mm_storeu_si128(lp.add(count) as *mut __m128i, idx);
                count += mb.count_ones() as usize;
            }
        }
    }

    let pairs = sqp as *const i32;
    let wp = w.as_ptr();
    let mut t = [zero; 4];
    let (mut a0, mut a1) = (zero, zero);
    let mut k = 0;
    while k < count {
        let end = (k + FLUSH_PAIRS).min(count);
        while k < end {
            let p = *lp.add(k) as usize;
            let cp = _mm256_set1_epi32(*pairs.add(p));
            let row = wp.add(p * 2 * L2) as *const __m256i;
            a0 = _mm256_add_epi32(a0, _mm256_madd_epi16(cp, _mm256_loadu_si256(row)));
            a1 = _mm256_add_epi32(a1, _mm256_madd_epi16(cp, _mm256_loadu_si256(row.add(1))));
            k += 1;
        }
        t[0] = _mm256_add_epi64(t[0], _mm256_cvtepi32_epi64(_mm256_castsi256_si128(a0)));
        t[1] = _mm256_add_epi64(t[1], _mm256_cvtepi32_epi64(_mm256_extracti128_si256::<1>(a0)));
        t[2] = _mm256_add_epi64(t[2], _mm256_cvtepi32_epi64(_mm256_castsi256_si128(a1)));
        t[3] = _mm256_add_epi64(t[3], _mm256_cvtepi32_epi64(_mm256_extracti128_si256::<1>(a1)));
        a0 = zero;
        a1 = zero;
    }
    for (k, v) in t.iter().enumerate() {
        _mm256_storeu_si256(out.as_mut_ptr().add(k * 4) as *mut __m256i, *v);
    }
}

#[cfg(target_arch = "aarch64")]
#[target_feature(enable = "neon")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn acc_addsub_neon(acc: &mut [i16], row: &[i16], sub: bool) {
    use std::arch::aarch64::*;
    let n = acc.len().min(row.len());
    let mut h = 0;
    while h + 8 <= n {
        let a = vld1q_s16(acc.as_ptr().add(h));
        let r = vld1q_s16(row.as_ptr().add(h));
        let s = if sub { vsubq_s16(a, r) } else { vaddq_s16(a, r) };
        vst1q_s16(acc.as_mut_ptr().add(h), s);
        h += 8;
    }
    if sub {
        while h < n { acc[h] = acc[h].wrapping_sub(row[h]); h += 1; }
    } else {
        while h < n { acc[h] = acc[h].wrapping_add(row[h]); h += 1; }
    }
}

pub fn refresh_one(
    net: &Net,
    sq_piece: &[Option<(Piece, Color)>; 64],
    perspective: Color,
    acc: &mut Accumulator,
) {
    let king_sq = sq_piece
        .iter()
        .position(|&s| s == Some((Piece::King, perspective)))
        .unwrap_or(0) as u8;
    let bucket = king_bucket(perspective, king_sq);
    acc.bucket = bucket;

    acc_set(&mut acc.v, &net.l0b);
    for (sq, slot) in sq_piece.iter().enumerate() {
        let Some((piece, color)) = *slot else { continue };
        let f = feature_index(perspective, bucket, piece, color, sq as u8);
        acc_add(&mut acc.v, weight_row(net, f));
    }

    #[cfg(debug_assertions)]
    {
        let mut wide = [0i32; HL];
        for h in 0..HL { wide[h] = net.l0b[h] as i32; }
        for (sq, slot) in sq_piece.iter().enumerate() {
            if let Some((piece, color)) = *slot {
                let f = feature_index(perspective, bucket, piece, color, sq as u8);
                let row = weight_row(net, f);
                for h in 0..HL { wide[h] += row[h] as i32; }
            }
        }
        for h in 0..HL {
            debug_assert!(
                wide[h] > i16::MIN as i32 + 256 && wide[h] < i16::MAX as i32 - 256,
                "NNUE hidden unit {h} = {} is within 256 of the i16 boundary -- this net is too \
                 hot for i16 accumulators", wide[h],
            );
            debug_assert_eq!(wide[h] as i16, acc.v[h], "i16 accumulator diverged at unit {h}");
        }
    }
}

#[derive(Clone)]
pub struct BucketCache {
    entries: Box<[CacheEntry; 2 * INPUT_BUCKETS]>,
}

#[derive(Clone)]
#[repr(C, align(64))]
struct CacheEntry {
    acc: [i16; HL],

    pieces: [[u64; 6]; 2],
    valid: bool,
}

impl BucketCache {
    pub fn new() -> Self {
        let empty = CacheEntry { acc: [0; HL], pieces: [[0; 6]; 2], valid: false };
        BucketCache { entries: Box::new(std::array::from_fn(|_| empty.clone())) }
    }

    pub fn clear(&mut self) {
        for e in self.entries.iter_mut() {
            e.valid = false;
        }
    }
}

pub fn refresh_cached(
    net: &Net,
    cache: &mut BucketCache,
    pieces: &[[u64; 6]; 2],
    perspective: Color,
    acc: &mut Accumulator,
) {
    let king_sq = pieces[perspective as usize][Piece::King as usize].trailing_zeros() as u8;
    let bucket = king_bucket(perspective, king_sq);
    let e = &mut cache.entries[perspective as usize * INPUT_BUCKETS + bucket as usize];
    if !e.valid {
        e.acc.copy_from_slice(&net.l0b);
        e.pieces = [[0; 6]; 2];
        e.valid = true;
    }
    for color in 0..2 {
        for piece in 0..6 {
            let now = pieces[color][piece];
            let old = e.pieces[color][piece];
            let mut added = now & !old;
            while added != 0 {
                let sq = added.trailing_zeros() as u8;
                added &= added - 1;
                acc_add(&mut e.acc, weight_row(net, feature_index_raw(perspective, bucket, piece, color, sq)));
            }
            let mut removed = old & !now;
            while removed != 0 {
                let sq = removed.trailing_zeros() as u8;
                removed &= removed - 1;
                acc_sub(&mut e.acc, weight_row(net, feature_index_raw(perspective, bucket, piece, color, sq)));
            }
            e.pieces[color][piece] = now;
        }
    }
    acc.v.copy_from_slice(&e.acc);
    acc.bucket = bucket;
}

pub fn update_from_parent(
    net: &Net,
    perspective: Color,
    parent: &Accumulator,
    child: &mut Accumulator,
    feats: &[(Piece, Color, u8, bool)],
) {
    let bucket = parent.bucket;
    let mut adds: [&[i16; HL]; 2] = [&[0; HL]; 2];
    let mut subs: [&[i16; HL]; 2] = [&[0; HL]; 2];
    let (mut na, mut ns) = (0, 0);
    for &(p, c, sq, add) in feats {
        let row: &[i16; HL] = weight_row(net, feature_index(perspective, bucket, p, c, sq))
            .try_into()
            .unwrap();
        if add && na < 2 {
            adds[na] = row;
            na += 1;
        } else if !add && ns < 2 {
            subs[ns] = row;
            ns += 1;
        } else {

            unreachable!("update_from_parent: unexpected feature list {feats:?}");
        }
    }
    child.bucket = bucket;
    let (src, dst) = (&parent.v, &mut child.v);
    match (na, ns) {
        (1, 1) => fused::<1, 1>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        (1, 2) => fused::<1, 2>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        (2, 2) => fused::<2, 2>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        (2, 1) => fused::<2, 1>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        (1, 0) => fused::<1, 0>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        (0, 1) => fused::<0, 1>(dst, src, adds[0], adds[1], subs[0], subs[1]),
        _ => fused::<0, 0>(dst, src, adds[0], adds[1], subs[0], subs[1]),
    }
}

#[inline(always)]
fn fused<const NA: usize, const NS: usize>(
    dst: &mut [i16; HL],
    src: &[i16; HL],
    a0: &[i16; HL],
    a1: &[i16; HL],
    s0: &[i16; HL],
    s1: &[i16; HL],
) {
    for h in 0..HL {
        let mut v = src[h];
        if NA > 0 { v = v.wrapping_add(a0[h]); }
        if NA > 1 { v = v.wrapping_add(a1[h]); }
        if NS > 0 { v = v.wrapping_sub(s0[h]); }
        if NS > 1 { v = v.wrapping_sub(s1[h]); }
        dst[h] = v;
    }
}

pub fn refresh_all(
    net: &Net,
    sq_piece: &[Option<(Piece, Color)>; 64],
    dual: &mut DualAccumulator,
) {
    refresh_one(net, sq_piece, Color::White, &mut dual.white);
    refresh_one(net, sq_piece, Color::Black, &mut dual.black);
}

#[inline(always)]
fn screlu_f(x: f32) -> f32 {
    let c = x.clamp(0.0, 1.0);
    c * c
}

pub fn evaluate_with_net(net: &Net, acc: &DualAccumulator, stm: Color, piece_count: u32) -> i32 {
    let (us, them) = match stm {
        Color::White => (&acc.white, &acc.black),
        Color::Black => (&acc.black, &acc.white),
    };
    let ob = output_bucket(piece_count);

    let mut s = [0i64; L2];
    l1_forward(&us.v, &them.v, &net.l1w_pairs[ob * L2 * 2 * HL..(ob + 1) * L2 * 2 * HL], &mut s);
    float_layers(net, ob, &s)
}

#[inline]
fn float_layers(net: &Net, ob: usize, s: &[i64; L2]) -> i32 {
    let l1b = &net.l1b[ob * L2..(ob + 1) * L2];
    let mut h2 = [0f32; L2];
    l1_activate(s, l1b, &mut h2);

    let l2w = &net.l2w_t[ob * L2 * L3..(ob + 1) * L2 * L3];
    let l3w = &net.l3w[ob * L3..(ob + 1) * L3];
    let mut x = [0f32; L3];
    x.copy_from_slice(&net.l2b[ob * L3..(ob + 1) * L3]);
    let mut prod = [0f32; L3];
    l2_l3(&mut x, l2w, &h2, l3w, &mut prod);

    let mut out = net.l3b[ob];
    for p in prod {
        out += p;
    }

    (out * SCALE as f32) as i32
}

#[inline]
fn l1_activate(s: &[i64; L2], l1b: &[f32], h2: &mut [f32; L2]) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return l1_activate_avx2(s, l1b, h2) }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    if std::is_x86_feature_detected!("avx2") {
        unsafe { return l1_activate_avx2(s, l1b, h2) }
    }
    #[allow(unreachable_code)]
    for o in 0..L2 {
        h2[o] = screlu_f((s[o] as f64 * L1_SCALE) as f32 + l1b[o]);
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn l1_activate_avx2(s: &[i64; L2], l1b: &[f32], h2: &mut [f32; L2]) {
    use std::arch::x86_64::*;
    debug_assert!(l1b.len() >= L2);

    let magic_i = _mm256_set1_epi64x(0x4338_0000_0000_0000);
    let magic_d = _mm256_set1_pd(f64::from_bits(0x4338_0000_0000_0000));
    let scale = _mm256_set1_pd(L1_SCALE);
    let (zero, one) = (_mm_setzero_ps(), _mm_set1_ps(1.0));
    for q in 0..L2 / 4 {
        let x = _mm256_loadu_si256(s.as_ptr().add(q * 4) as *const __m256i);
        let d = _mm256_sub_pd(_mm256_castsi256_pd(_mm256_add_epi64(x, magic_i)), magic_d);
        let f = _mm_add_ps(_mm256_cvtpd_ps(_mm256_mul_pd(d, scale)), _mm_loadu_ps(l1b.as_ptr().add(q * 4)));
        let c = _mm_min_ps(_mm_max_ps(f, zero), one);
        _mm_storeu_ps(h2.as_mut_ptr().add(q * 4), _mm_mul_ps(c, c));
    }
}

#[inline]
fn l2_l3(x: &mut [f32; L3], w: &[f32], h: &[f32; L2], l3w: &[f32], prod: &mut [f32; L3]) {
    #[cfg(all(target_arch = "x86_64", target_feature = "avx2"))]
    unsafe { return l2_l3_avx2(x, w, h, l3w, prod) }
    #[cfg(all(target_arch = "x86_64", not(target_feature = "avx2")))]
    if std::is_x86_feature_detected!("avx2") {
        unsafe { return l2_l3_avx2(x, w, h, l3w, prod) }
    }
    #[allow(unreachable_code)]
    {
        for o in 0..L2 {
            let w = &w[o * L3..(o + 1) * L3];
            for j in 0..L3 {
                x[j] += w[j] * h[o];
            }
        }
        for j in 0..L3 {
            prod[j] = l3w[j] * screlu_f(x[j]);
        }
    }
}

#[cfg(target_arch = "x86_64")]
#[target_feature(enable = "avx2")]
#[allow(unsafe_op_in_unsafe_fn)]
unsafe fn l2_l3_avx2(x: &mut [f32; L3], w: &[f32], h: &[f32; L2], l3w: &[f32], prod: &mut [f32; L3]) {
    use std::arch::x86_64::*;
    debug_assert!(w.len() >= L2 * L3 && l3w.len() >= L3);
    let mut v = [_mm256_setzero_ps(); L3 / 8];
    for (q, vq) in v.iter_mut().enumerate() {
        *vq = _mm256_loadu_ps(x.as_ptr().add(q * 8));
    }
    for (o, &ho) in h.iter().enumerate() {
        let hb = _mm256_set1_ps(ho);
        let row = w.as_ptr().add(o * L3);
        for (q, vq) in v.iter_mut().enumerate() {
            *vq = _mm256_add_ps(*vq, _mm256_mul_ps(_mm256_loadu_ps(row.add(q * 8)), hb));
        }
    }
    let (zero, one) = (_mm256_setzero_ps(), _mm256_set1_ps(1.0));
    for (q, vq) in v.iter().enumerate() {
        _mm256_storeu_ps(x.as_mut_ptr().add(q * 8), *vq);
        let c = _mm256_min_ps(_mm256_max_ps(*vq, zero), one);
        let p = _mm256_mul_ps(_mm256_loadu_ps(l3w.as_ptr().add(q * 8)), _mm256_mul_ps(c, c));
        _mm256_storeu_ps(prod.as_mut_ptr().add(q * 8), p);
    }
}

pub fn evaluate(board: &mut Board) -> i32 {
    let hash = board.hash;
    if let Some(e) = board.eval_cache().probe(hash) {
        return e;
    }
    let stm = board.side;
    let piece_count = board.occupied().count_ones();
    let (net, acc) = board.nnue_state().expect("NNUE net not loaded");
    let e = evaluate_with_net(net, acc, stm, piece_count);
    board.eval_cache().store(hash, e);
    e
}

#[cfg(test)]
mod tests {
    use super::*;

    fn board_from(fen: &str) -> Board {
        Board::from_fen(fen).expect("valid fen")
    }

    #[test]
    fn embedded_net_parses() {

        assert!(parse_net(EMBEDDED_NET).is_ok());
    }

    #[test]
    fn output_bucket_matches_material_count() {
        for count in 2..=32u32 {
            assert_eq!(output_bucket(count), ((count - 2) / 4) as usize, "count {count}");
        }
    }

    #[test]
    fn king_bucket_is_perspective_relative() {
        for sq in 0..64u8 {
            assert_eq!(king_bucket(Color::White, sq), king_bucket(Color::Black, sq ^ 56));
        }
        assert_eq!(king_bucket(Color::White, 6), 1);
        assert_eq!(king_bucket(Color::Black, 62), 1);
    }

    #[test]
    fn simd_matches_scalar() {

        let mut s: u64 = 0x9E3779B97F4A7C15;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };

        for trial in 0..64 {

            let mut acc_ref = [0i16; HL];
            let mut acc_simd = [0i16; HL];
            for h in 0..HL {
                let v = ((next() % 4096) as i32 - 1024) as i16;
                acc_ref[h] = v;
                acc_simd[h] = v;
            }
            let row: Vec<i16> = (0..HL).map(|_| (next() % 512) as i16 - 256).collect();

            let sub = trial & 1 == 0;
            acc_addsub_scalar(&mut acc_ref, &row, sub);
            if sub { acc_sub(&mut acc_simd, &row) } else { acc_add(&mut acc_simd, &row) }
            assert_eq!(acc_ref, acc_simd, "acc_{} mismatch on trial {trial}", if sub {"sub"} else {"add"});
        }
    }

    #[test]
    fn l1_forward_matches_dense() {
        let mut s: u64 = 0x2545F4914F6CDD1D;
        let mut next = || {
            s ^= s << 13;
            s ^= s >> 7;
            s ^= s << 17;
            s
        };
        let dense = |c: &[i16; 2 * HL], w: &[i16]| -> [i64; L2] {
            let mut out = [0i64; L2];
            for o in 0..L2 {
                for i in 0..2 * HL {
                    let (p, k) = (i / 2, i % 2);
                    out[o] += c[i] as i64 * w[(p * L2 + o) * 2 + k] as i64;
                }
            }
            out
        };
        let halves = |a: &[i16; 2 * HL]| -> ([i16; HL], [i16; HL]) {
            (a[..HL].try_into().unwrap(), a[HL..].try_into().unwrap())
        };
        for trial in 0..40 {
            let zero_pct = [0u64, 50, 88, 99, 100][trial % 5];
            let mut acc = [0i16; 2 * HL];
            for v in acc.iter_mut() {
                *v = if next() % 100 < zero_pct { -5 } else { (next() % 300) as i16 };
            }
            let mut c = [0i16; 2 * HL];
            screlu_half(&acc, &mut c);
            let w: Vec<i16> = (0..2 * HL * L2).map(|_| (next() % 257) as i16 - 128).collect();
            let want = dense(&c, &w);
            let mut got = [0i64; L2];
            l1_sparse_scalar(&c, &w, &mut got);
            assert_eq!(got, want, "scalar, trial {trial}");
            let (us, them) = halves(&acc);
            l1_forward(&us, &them, &w, &mut got);
            assert_eq!(got, want, "dispatched, trial {trial}");
        }
        let w = vec![128i16; 2 * HL * L2];
        let mut got = [0i64; L2];
        l1_forward(&[QA_I16; HL], &[QA_I16; HL], &w, &mut got);
        assert_eq!(got, [2 * HL as i64 * ((QA * QA) >> 1) * 128; L2]);
    }

    #[test]
    fn l1_activate_matches_scalar() {
        let mut st: u64 = 0x1234_5678_9ABC_DEF1;
        let mut next = || {
            st ^= st << 13;
            st ^= st >> 7;
            st ^= st << 17;
            st
        };
        for trial in 0..2000 {
            let span = [1i64 << 12, 1 << 22, 1 << 33][trial % 3];
            let s: [i64; L2] = std::array::from_fn(|_| (next() % (2 * span as u64)) as i64 - span);
            let l1b: Vec<f32> = (0..L2).map(|_| (next() % 2000) as f32 / 1000.0 - 1.0).collect();
            let mut got = [0f32; L2];
            l1_activate(&s, &l1b, &mut got);
            for o in 0..L2 {
                let want = screlu_f((s[o] as f64 * L1_SCALE) as f32 + l1b[o]);
                assert_eq!(got[o].to_bits(), want.to_bits(), "s={} b={}", s[o], l1b[o]);
            }
        }
    }

    #[test]
    fn forward_pass_matches_scalar() {
        load_embedded();
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R b KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
        ] {
            let mut b = board_from(fen);
            let stm = b.side;
            let ob = output_bucket(b.occupied().count_ones());
            let (net, acc) = b.nnue_state().unwrap();
            let (us, them) = match stm {
                Color::White => (&acc.white, &acc.black),
                Color::Black => (&acc.black, &acc.white),
            };
            let sq = |a: i16| -> i64 {
                let c = a.clamp(0, QA_I16) as i64;
                (c * c) >> 1
            };
            let mut h2 = [0f32; L2];

            let w = |o: usize, i: usize| net.l1w_pairs[((ob * HL + i / 2) * L2 + o) * 2 + i % 2] as i64;
            for o in 0..L2 {
                let mut s: i64 = 0;
                for h in 0..HL {
                    s += sq(us.v[h]) * w(o, h) + sq(them.v[h]) * w(o, HL + h);
                }
                h2[o] = screlu_f((s as f64 * L1_SCALE) as f32 + net.l1b[ob * L2 + o]);
            }
            let mut out = net.l3b[ob];
            for j in 0..L3 {
                let mut x = net.l2b[ob * L3 + j];
                for o in 0..L2 {
                    x += net.l2w_t[(ob * L2 + o) * L3 + j] * h2[o];
                }
                out += net.l3w[ob * L3 + j] * screlu_f(x);
            }
            let want = (out * SCALE as f32) as i32;
            assert_eq!(want, evaluate(&mut b), "forward pass mismatch on {fen}");
        }
    }

    #[test]
    #[ignore]
    fn bench_eval() {
        use std::hint::black_box;
        use std::time::Instant;
        load_embedded();
        let path = std::env::var("EREBUS_BENCH_FENS").expect("set EREBUS_BENCH_FENS");
        let mut boards: Vec<Board> = std::fs::read_to_string(path)
            .unwrap()
            .lines()
            .filter_map(Board::from_fen)
            .take(64)
            .collect();
        let net_arc = current_net().unwrap();
        let net = &*net_arc;
        let reps = 2000;
        let n = (reps * boards.len()) as f64;
        let mut time = |name: &str, f: &mut dyn FnMut(&mut Board)| {
            let t = Instant::now();
            for _ in 0..reps {
                for b in boards.iter_mut() {
                    f(black_box(b));
                }
            }
            println!("{name:<13}: {:7.1} ns", t.elapsed().as_nanos() as f64 / n);
        };

        time("forward pass", &mut |b| {
            black_box(evaluate(b));
        });
        time("  screlu", &mut |b| {
            let mut c = [0i16; 2 * HL];
            let (_, acc) = b.nnue_state().unwrap();
            screlu_half(&acc.white.v, &mut c[..HL]);
            screlu_half(&acc.black.v, &mut c[HL..]);
            black_box(&c);
        });
        time("  screlu+l1", &mut |b| {
            let ob = output_bucket(b.occupied().count_ones());
            let (_, acc) = b.nnue_state().unwrap();
            let mut s = [0i64; L2];
            l1_forward(&acc.white.v, &acc.black.v, &net.l1w_pairs[ob * L2 * 2 * HL..(ob + 1) * L2 * 2 * HL], &mut s);
            black_box(&s);
        });
        time("full rebuild", &mut |b| {
            let mut acc = Accumulator::zeroed();
            refresh_one(net, &b.sq_piece, Color::White, &mut acc);
            black_box(&acc);
        });
        let parent = Accumulator::zeroed();
        let mut child = Accumulator::zeroed();
        let mut i = 0u8;
        time("quiet move", &mut |_| {
            i = i.wrapping_add(1);
            let feats = [(Piece::Knight, Color::White, i % 64, false), (Piece::Knight, Color::White, (i + 17) % 64, true)];
            update_from_parent(net, Color::White, &parent, &mut child, &feats);
            black_box(&child);
        });
    }

    fn assert_acc_fresh(b: &mut Board) {
        let sq_piece = b.sq_piece;
        let (net, acc) = b.nnue_state().unwrap();
        let mut want = DualAccumulator::zeroed();
        refresh_all(net, &sq_piece, &mut want);
        assert!(*acc == want, "lazy accumulator differs from a full refresh");
    }

    fn lazy_walk(b: &mut Board, depth: u32, visited: &mut u64) {
        use crate::movegen::MoveGen;
        use crate::moves::MoveList;

        *visited += 1;
        if depth == 0 || visited.is_multiple_of(3) {
            assert_acc_fresh(b);
        }
        if depth == 0 {
            return;
        }
        let mut moves = MoveList::new();
        MoveGen::generate_all(b, &mut moves);
        for i in 0..moves.len() {
            b.make_move(moves[i]);
            lazy_walk(b, depth - 1, visited);
            b.unmake_move();
        }
        if visited.is_multiple_of(5) {
            assert_acc_fresh(b);
        }
    }

    #[test]
    fn lazy_accumulators_match_refresh() {
        load_embedded();
        for (fen, depth) in [

            ("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1", 3),

            ("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8", 3),

            ("8/8/3k4/8/8/4K3/8/R6r w - - 0 1", 4),
        ] {
            let mut b = board_from(fen);
            let mut visited = 0;
            lazy_walk(&mut b, depth, &mut visited);
            assert_acc_fresh(&mut b);
        }
    }

    #[test]
    fn lazy_accumulators_survive_a_long_line() {
        load_embedded();
        let mut b = board_from("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let shuffle = ["g1f3", "g8f6", "f3g1", "f6g8"];
        let plies = 300;
        for i in 0..plies {
            let mv = b.find_uci_move(shuffle[i % 4]).expect("legal shuffle move");
            b.make_move(mv);
            if i % 50 == 0 {
                assert_acc_fresh(&mut b);
            }
        }
        assert_acc_fresh(&mut b);
        for i in 0..plies {
            b.unmake_move();
            if i % 7 == 0 {
                assert_acc_fresh(&mut b);
            }
        }
        assert_acc_fresh(&mut b);
    }

    #[test]
    fn startpos_eval_is_small_and_symmetric_ish() {
        load_embedded();
        let mut start = board_from("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let e = evaluate(&mut start);

        assert!(e.abs() < 150, "startpos eval {e} out of sane range");
    }

    #[test]
    fn massive_material_advantage_is_detected() {
        load_embedded();

        let mut w = board_from("4k3/8/8/8/8/8/8/QQQQK3 w - - 0 1");
        let e = evaluate(&mut w);
        assert!(e > 800, "QQQQ vs k should be winning for White, got {e}");

        let mut b = board_from("4k3/8/8/8/8/8/8/QQQQK3 b - - 0 1");
        let e = evaluate(&mut b);
        assert!(e < -800, "QQQQ vs k, Black to move, got {e}");
    }

    #[test]
    fn color_flip_symmetry() {
        load_embedded();
        let a = evaluate(&mut board_from("r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R w KQkq - 0 1"));
        let b = evaluate(&mut board_from("r1bqk2r/pppp1ppp/2n2n2/2b1p3/2B1P3/2N2N2/PPPP1PPP/R1BQK2R b KQkq - 0 1"));

        assert!(a.abs() < 200 && b.abs() < 200, "symmetric position evals {a} / {b} not small");
    }
}
