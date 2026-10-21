

use std::fmt;
use std::io;
use std::path::Path;

pub const MAGIC: &[u8; 8] = b"FBKBOOK\0";
pub const VERSION: u16 = 1;
pub const HEADER_SIZE: usize = 64;
pub const ENTRY_SIZE: usize = 32;

pub const FLAG_CHESS960: u16 = 1;

pub const FLAG_STANDARD: u16 = 2;

pub const ENTRY_SEARCHED: u16 = 1;

pub const EVAL_NONE: i16 = i16::MIN;

pub const EVAL_MATE: i16 = 32000;
pub const EVAL_MATE_MIN: i16 = 30000;

const SEED: u64 = 0x4642_4B42_4F4F_4B31;

const fn gen_keys() -> [u64; 793] {
    let mut keys = [0u64; 793];
    let mut state = SEED;
    let mut i = 0;
    while i < 793 {
        state = state.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = state;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        keys[i] = z ^ (z >> 31);
        i += 1;
    }
    keys
}

pub static KEYS: [u64; 793] = gen_keys();
const CASTLE_BASE: usize = 768;
const EP_BASE: usize = 784;
const BLACK_INDEX: usize = 792;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Color {
    White = 0,
    Black = 1,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Piece {
    Pawn = 0,
    Knight = 1,
    Bishop = 2,
    Rook = 3,
    Queen = 4,
    King = 5,
}

impl Piece {
    fn from_char(c: char) -> Option<(Color, Piece)> {
        let color = if c.is_ascii_uppercase() { Color::White } else { Color::Black };
        let piece = match c.to_ascii_lowercase() {
            'p' => Piece::Pawn,
            'n' => Piece::Knight,
            'b' => Piece::Bishop,
            'r' => Piece::Rook,
            'q' => Piece::Queen,
            'k' => Piece::King,
            _ => return None,
        };
        Some((color, piece))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Position {
    pub board: [Option<(Color, Piece)>; 64],
    pub black_to_move: bool,

    pub castling_rooks: u64,

    pub ep_square: Option<u8>,
}

impl Position {

    pub fn from_fen(fen: &str) -> Result<Position, String> {
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() < 4 {
            return Err(format!("FEN needs at least 4 fields: {fen:?}"));
        }
        let mut board = [None; 64];
        let ranks: Vec<&str> = parts[0].split('/').collect();
        if ranks.len() != 8 {
            return Err(format!("FEN board needs 8 ranks: {fen:?}"));
        }
        for (i, rank_text) in ranks.iter().enumerate() {
            let rank = 7 - i;
            let mut file = 0usize;
            for c in rank_text.chars() {
                if let Some(d) = c.to_digit(10) {
                    file += d as usize;
                } else {
                    let pc = Piece::from_char(c).ok_or(format!("bad piece {c:?}"))?;
                    if file > 7 {
                        return Err(format!("rank too long: {rank_text:?}"));
                    }
                    board[rank * 8 + file] = Some(pc);
                    file += 1;
                }
            }
            if file != 8 {
                return Err(format!("rank has {file} files: {rank_text:?}"));
            }
        }

        let black_to_move = match parts[1] {
            "w" => false,
            "b" => true,
            s => return Err(format!("bad side to move {s:?}")),
        };

        let mut castling_rooks = 0u64;
        if parts[2] != "-" {
            for c in parts[2].chars() {
                let color = if c.is_ascii_uppercase() { Color::White } else { Color::Black };
                let base = if color == Color::White { 0 } else { 56 };
                let Some(king_file) = (0..8).find(|&f| board[base + f] == Some((color, Piece::King)))
                else {
                    continue;
                };
                let rook_on = |f: usize| board[base + f] == Some((color, Piece::Rook));
                let rook_file = match c.to_ascii_lowercase() {
                    'k' => (king_file + 1..8).rev().find(|&f| rook_on(f)),
                    'q' => (0..king_file).find(|&f| rook_on(f)),
                    f @ 'a'..='h' => Some(f as usize - 'a' as usize).filter(|&f| rook_on(f)),
                    _ => return Err(format!("bad castling field {:?}", parts[2])),
                };
                if let Some(f) = rook_file {
                    castling_rooks |= 1u64 << (base + f);
                }
            }
        }

        let ep_square = match parts[3] {
            "-" => None,
            s => Some(parse_square(s).ok_or(format!("bad en passant square {s:?}"))?),
        };

        Ok(Position { board, black_to_move, castling_rooks, ep_square })
    }

    pub fn key(&self) -> u64 {
        let mut key = 0u64;
        for (sq, cell) in self.board.iter().enumerate() {
            if let Some((color, piece)) = cell {
                key ^= KEYS[(*color as usize * 6 + *piece as usize) * 64 + sq];
            }
        }

        let mut rooks = self.castling_rooks;
        while rooks != 0 {
            let sq = rooks.trailing_zeros() as usize;
            rooks &= rooks - 1;
            let color = if sq < 8 { 0 } else { 1 };
            key ^= KEYS[CASTLE_BASE + color * 8 + (sq & 7)];
        }

        if let Some(ep) = self.ep_square {
            if self.ep_capturable(ep as usize) {
                key ^= KEYS[EP_BASE + (ep as usize & 7)];
            }
        }

        if self.black_to_move {
            key ^= KEYS[BLACK_INDEX];
        }
        key
    }

    fn ep_capturable(&self, ep: usize) -> bool {
        let (us, rank_offset) = if self.black_to_move {
            (Color::Black, 8isize)
        } else {
            (Color::White, -8isize)
        };
        let file = (ep & 7) as isize;
        [-1isize, 1].iter().any(|df| {
            let f = file + df;
            let sq = ep as isize + rank_offset + df;
            (0..8).contains(&f)
                && (0..64).contains(&sq)
                && self.board[sq as usize] == Some((us, Piece::Pawn))
        })
    }
}

pub fn key_from_fen(fen: &str) -> Result<u64, String> {
    Position::from_fen(fen).map(|p| p.key())
}

fn parse_square(s: &str) -> Option<u8> {
    let b = s.as_bytes();
    if b.len() != 2 || !(b'a'..=b'h').contains(&b[0]) || !(b'1'..=b'8').contains(&b[1]) {
        return None;
    }
    Some((b[1] - b'1') * 8 + (b[0] - b'a'))
}

fn square_name(sq: u8) -> String {
    format!("{}{}", (b'a' + (sq & 7)) as char, (b'1' + (sq >> 3)) as char)
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct BookMove(pub u16);

impl BookMove {
    pub fn from(self) -> u8 {
        ((self.0 >> 6) & 63) as u8
    }

    pub fn to(self) -> u8 {
        (self.0 & 63) as u8
    }

    pub fn promotion(self) -> Option<Piece> {
        match (self.0 >> 12) & 7 {
            1 => Some(Piece::Knight),
            2 => Some(Piece::Bishop),
            3 => Some(Piece::Rook),
            4 => Some(Piece::Queen),
            _ => None,
        }
    }

    pub fn is_castling(self, pos: &Position) -> bool {
        match (pos.board[self.from() as usize], pos.board[self.to() as usize]) {
            (Some((c1, Piece::King)), Some((c2, Piece::Rook))) => c1 == c2,
            _ => false,
        }
    }

    pub fn uci(self, pos: &Position, chess960: bool) -> String {
        let mut to = self.to();
        if !chess960 && self.is_castling(pos) {
            let rank = self.from() & !7;
            to = rank + if self.to() > self.from() { 6 } else { 2 };
        }
        let promo = match self.promotion() {
            Some(Piece::Knight) => "n",
            Some(Piece::Bishop) => "b",
            Some(Piece::Rook) => "r",
            Some(Piece::Queen) => "q",
            _ => "",
        };
        format!("{}{}{}", square_name(self.from()), square_name(to), promo)
    }
}

impl fmt::Display for BookMove {

    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let promo = match self.promotion() {
            Some(Piece::Knight) => "n",
            Some(Piece::Bishop) => "b",
            Some(Piece::Rook) => "r",
            Some(Piece::Queen) => "q",
            _ => "",
        };
        write!(f, "{}{}{}", square_name(self.from()), square_name(self.to()), promo)
    }
}

// ── Book ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Entry {
    pub key: u64,
    pub mv: BookMove,
    pub flags: u16,
    pub wins: u32,
    pub draws: u32,
    pub losses: u32,
    /// Raw eval field; see [`Entry::eval`].
    pub eval_raw: i16,
    pub depth: u8,
}

impl Entry {
    pub fn games(&self) -> u32 {
        self.wins + self.draws + self.losses
    }

    /// Eval after the move in centipawns from the mover's view, if known.

    pub fn eval(&self) -> Option<i16> {
        (self.eval_raw != EVAL_NONE).then_some(self.eval_raw)
    }

    pub fn score(&self) -> f64 {
        let n = self.games();
        if n == 0 {
            0.5
        } else {
            (self.wins as f64 + 0.5 * self.draws as f64) / n as f64
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Policy {

    pub min_games: u32,

    pub max_eval_drop: Option<i32>,
}

impl Default for Policy {
    fn default() -> Self {
        Policy { min_games: 1, max_eval_drop: Some(50) }
    }
}

pub struct Book {
    data: Vec<u8>,
    pub flags: u16,
    pub game_count: u64,
    entry_size: usize,
    count: usize,
    offset: usize,
    meta_len: usize,
}

fn invalid(msg: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, msg.to_string())
}

fn u16_at(d: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([d[o], d[o + 1]])
}

fn u32_at(d: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(d[o..o + 4].try_into().unwrap())
}

fn u64_at(d: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(d[o..o + 8].try_into().unwrap())
}

impl Book {
    pub fn open(path: impl AsRef<Path>) -> io::Result<Book> {
        Book::from_bytes(std::fs::read(path)?)
    }

    pub fn from_bytes(data: Vec<u8>) -> io::Result<Book> {
        if data.len() < HEADER_SIZE || &data[0..8] != MAGIC {
            return Err(invalid("not an FBK book"));
        }
        if u16_at(&data, 8) != VERSION {
            return Err(invalid("unsupported FBK version"));
        }
        let flags = u16_at(&data, 10);
        let entry_size = u16_at(&data, 12) as usize;
        let count = u64_at(&data, 16) as usize;
        let game_count = u64_at(&data, 24);
        let offset = u64_at(&data, 32) as usize;
        let meta_len = u32_at(&data, 40) as usize;
        if entry_size < ENTRY_SIZE {
            return Err(invalid("bad entry size"));
        }
        let end = count
            .checked_mul(entry_size)
            .and_then(|n| n.checked_add(offset))
            .ok_or_else(|| invalid("bad entry count"))?;
        if end > data.len() || HEADER_SIZE + meta_len > data.len() {
            return Err(invalid("truncated FBK book"));
        }
        Ok(Book { data, flags, game_count, entry_size, count, offset, meta_len })
    }

    pub fn len(&self) -> usize {
        self.count
    }

    pub fn is_empty(&self) -> bool {
        self.count == 0
    }

    pub fn metadata(&self) -> &str {
        std::str::from_utf8(&self.data[HEADER_SIZE..HEADER_SIZE + self.meta_len]).unwrap_or("")
    }

    fn key_at(&self, i: usize) -> u64 {
        u64_at(&self.data, self.offset + i * self.entry_size)
    }

    pub fn entry(&self, i: usize) -> Entry {
        let o = self.offset + i * self.entry_size;
        let d = &self.data;
        Entry {
            key: u64_at(d, o),
            mv: BookMove(u16_at(d, o + 8)),
            flags: u16_at(d, o + 10),
            wins: u32_at(d, o + 12),
            draws: u32_at(d, o + 16),
            losses: u32_at(d, o + 20),
            eval_raw: u16_at(d, o + 24) as i16,
            depth: d[o + 26],
        }
    }

    pub fn probe(&self, key: u64) -> Vec<Entry> {
        let (mut lo, mut hi) = (0, self.count);
        while lo < hi {
            let mid = (lo + hi) / 2;
            if self.key_at(mid) < key {
                lo = mid + 1;
            } else {
                hi = mid;
            }
        }
        (lo..self.count)
            .take_while(|&i| self.key_at(i) == key)
            .map(|i| self.entry(i))
            .collect()
    }

    pub fn choose(&self, key: u64, policy: &Policy, random: u64) -> Option<BookMove> {
        let mut cands: Vec<Entry> =
            self.probe(key).into_iter().filter(|e| e.games() >= policy.min_games).collect();
        if let Some(drop) = policy.max_eval_drop {
            if let Some(best) = cands.iter().filter_map(|e| e.eval()).max() {
                let floor = best as i32 - drop;
                cands.retain(|e| e.eval().map_or(true, |v| v as i32 >= floor));
            }
        }
        let total: u64 = cands.iter().map(|e| e.games() as u64).sum();
        if total == 0 {
            return None;
        }
        let mut r = random % total;
        for e in &cands {
            let g = e.games() as u64;
            if r < g {
                return Some(e.mv);
            }
            r -= g;
        }
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spec_check_values() {
        assert_eq!(KEYS[0], 0xB403_228A_45B2_56D8);
        assert_eq!(KEYS[1], 0x0E07_55AE_F065_ACCD);
        assert_eq!(KEYS[792], 0x5497_46F0_CB0B_F37E);
    }

    #[test]
    fn xfen_and_shredder_agree() {
        let a = key_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1");
        let b = key_from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w HAha - 0 1");
        assert_eq!(a, b);
    }

    #[test]
    fn ep_only_counts_when_capturable() {

        let with = key_from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1");
        let without = key_from_fen("rnbqkbnr/pppppppp/8/8/4P3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        assert_eq!(with, without);

        let with = key_from_fen("rnbqkbnr/ppp1pppp/8/8/3pP3/8/PPPP1PPP/RNBQKBNR b KQkq e3 0 1");
        let without = key_from_fen("rnbqkbnr/ppp1pppp/8/8/3pP3/8/PPPP1PPP/RNBQKBNR b KQkq - 0 1");
        assert_ne!(with, without);
    }

    #[test]
    fn castling_uci_conversion() {
        let pos = Position::from_fen("rnbqk2r/pppppppp/8/8/8/8/PPPPPPPP/RNBQK2R w KQkq - 0 1").unwrap();
        let mv = BookMove((4 << 6) | 7);
        assert!(mv.is_castling(&pos));
        assert_eq!(mv.uci(&pos, true), "e1h1");
        assert_eq!(mv.uci(&pos, false), "e1g1");
    }
}
