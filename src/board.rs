use std::sync::Arc;

use crate::bitboard::*;
use crate::moves::{Move, MoveFlag};
use crate::zobrist::ZOBRIST;

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Color { White = 0, Black = 1 }

impl Color {
    #[inline] pub fn flip(self) -> Self {
        if self == Color::White { Color::Black } else { Color::White }
    }
}

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum Piece { Pawn = 0, Knight = 1, Bishop = 2, Rook = 3, Queen = 4, King = 5 }

impl Piece {
    pub fn from_char(c: char) -> Option<(Piece, Color)> {
        let color = if c.is_uppercase() { Color::White } else { Color::Black };
        let piece = match c.to_ascii_lowercase() {
            'p' => Piece::Pawn,
            'n' => Piece::Knight,
            'b' => Piece::Bishop,
            'r' => Piece::Rook,
            'q' => Piece::Queen,
            'k' => Piece::King,
            _   => return None,
        };
        Some((piece, color))
    }
    pub fn to_char(self, color: Color) -> char {
        let c = match self {
            Piece::Pawn   => 'p',
            Piece::Knight => 'n',
            Piece::Bishop => 'b',
            Piece::Rook   => 'r',
            Piece::Queen  => 'q',
            Piece::King   => 'k',
        };
        if color == Color::White { c.to_ascii_uppercase() } else { c }
    }
}

pub static CHESS960: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

#[derive(Copy, Clone, Default)]
pub struct CastleInfo {
    pub king:    u8,
    pub rook:    u8,
    pub king_to: u8,
    pub rook_to: u8,

    pub empty:   Bitboard,

    pub safe:    Bitboard,
}

#[inline]
pub fn castle_index(us: Color, flags: MoveFlag) -> usize {
    us as usize * 2 + (flags == MoveFlag::CastleQueen) as usize
}

use crate::nnue::FeatDelta;

#[derive(Copy, Clone)]
pub struct IrrevState {
    pub captured:    Option<Piece>,
    pub ep_sq:       Option<u8>,
    pub castling:    u8,
    pub halfmove:    u16,
    pub hash:        u64,
}

#[derive(Clone)]
pub struct Board {

    pub pieces:   [[Bitboard; 6]; 2],

    pub color_bb: [Bitboard; 2],

    pub sq_piece: [Option<(Piece, Color)>; 64],

    pub side:       Color,
    pub ep_sq:      Option<u8>,
    pub castling:   u8,
    pub halfmove:   u16,
    pub fullmove:   u16,
    pub hash:       u64,

    pub castle:      [CastleInfo; 4],
    castle_mask:     [u8; 64],

    pub history:    Vec<(Move, IrrevState)>,

    nnue_stack:      crate::nnue::AccStack,
    net:             Option<Arc<crate::nnue::Net>>,

    nnue_cache:      crate::nnue::BucketCache,

    eval_cache:      crate::nnue::EvalCache,
}

impl Board {
    pub fn empty() -> Self {
        Board {
            pieces:   [[0; 6]; 2],
            color_bb: [0; 2],
            sq_piece: [None; 64],
            side:     Color::White,
            ep_sq:    None,
            castling: 0,
            halfmove: 0,
            fullmove: 1,
            hash:     0,
            castle:      [CastleInfo::default(); 4],
            castle_mask: [0xFF; 64],
            history:  Vec::new(),
            nnue_stack:  crate::nnue::AccStack::new(),
            net:         crate::nnue::current_net(),
            nnue_cache:  crate::nnue::BucketCache::new(),
            eval_cache:  crate::nnue::EvalCache::new(),
        }
    }

    pub fn start_pos() -> Self {
        Self::from_fen("rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1").unwrap()
    }

    #[inline] pub fn occupied(&self) -> Bitboard { self.color_bb[0] | self.color_bb[1] }

    #[inline]
    pub fn piece_bb(&self, color: Color, piece: Piece) -> Bitboard {
        self.pieces[color as usize][piece as usize]
    }

    #[inline]
    fn set_piece(&mut self, sq: u8, piece: Piece, color: Color) {
        let b = bit(sq);
        self.pieces[color as usize][piece as usize] |= b;
        self.color_bb[color as usize] |= b;
        self.sq_piece[sq as usize] = Some((piece, color));
        self.hash ^= ZOBRIST.piece[color as usize][piece as usize][sq as usize];
    }

    #[inline]
    fn clear_piece(&mut self, sq: u8) {
        if let Some((piece, color)) = self.sq_piece[sq as usize] {
            let b = bit(sq);
            self.pieces[color as usize][piece as usize] &= !b;
            self.color_bb[color as usize] &= !b;
            self.sq_piece[sq as usize] = None;
            self.hash ^= ZOBRIST.piece[color as usize][piece as usize][sq as usize];
        }
    }

    #[inline]
    pub fn nnue_state(&mut self) -> Option<(&crate::nnue::Net, &crate::nnue::DualAccumulator)> {
        let net = self.net.as_deref()?;
        let acc = self.nnue_stack.current(net, &mut self.nnue_cache);

        #[cfg(debug_assertions)]
        {
            let mut check = crate::nnue::DualAccumulator::zeroed();
            crate::nnue::refresh_all(net, &self.sq_piece, &mut check);
            debug_assert_eq!(check.white, acc.white, "lazy NNUE accumulator (White) desynced from a full refresh");
            debug_assert_eq!(check.black, acc.black, "lazy NNUE accumulator (Black) desynced from a full refresh");
        }

        Some((net, acc))
    }

    #[inline]
    pub(crate) fn eval_cache(&self) -> &crate::nnue::EvalCache {
        &self.eval_cache
    }

    pub fn reload_net(&mut self) {
        self.net = crate::nnue::current_net();
        self.nnue_cache.clear();
        self.eval_cache.clear();
        self.refresh_nnue();
    }

    pub fn refresh_nnue(&mut self) {
        let Some(net) = self.net.as_deref() else { return };
        self.nnue_stack.reset(net, &self.sq_piece);
    }

    pub fn from_fen(fen: &str) -> Option<Self> {
        let mut board = Board::empty();
        let parts: Vec<&str> = fen.split_whitespace().collect();
        if parts.len() < 4 { return None; }

        let mut rank: i32 = 7;
        let mut file: i32 = 0;
        for c in parts[0].chars() {
            match c {
                '/' => { rank -= 1; file = 0; }
                '1'..='8' => { file += (c as i32) - ('0' as i32); }
                _ => {
                    if let Some((piece, color)) = Piece::from_char(c) {
                        let sq = (rank * 8 + file) as u8;
                        board.set_piece(sq, piece, color);
                        file += 1;
                    }
                }
            }
        }

        board.side = if parts[1] == "b" { Color::Black } else { Color::White };

        board.castling = 0;
        for c in parts[2].chars() {
            let color = if c.is_ascii_uppercase() { Color::White } else { Color::Black };
            let base  = if color == Color::White { 0u8 } else { 56 };
            let kings = board.pieces[color as usize][Piece::King as usize] & (0xFFu64 << base);
            if kings == 0 { continue; }
            let king  = lsb(kings);
            let kfile = king & 7;
            let rook_on = |f: u8| board.sq_piece[(base + f) as usize] == Some((Piece::Rook, color));
            let rook_file = match c.to_ascii_lowercase() {
                'k' => (kfile + 1..8).rev().find(|&f| rook_on(f)).unwrap_or(7),
                'q' => (0..kfile).find(|&f| rook_on(f)).unwrap_or(0),
                f @ 'a'..='h' => f as u8 - b'a',
                _ => continue,
            };
            if rook_file == kfile { continue; }
            let queen_side = rook_file < kfile;
            let idx  = color as usize * 2 + queen_side as usize;
            let rook = base + rook_file;
            let (king_to, rook_to) = if queen_side { (base + 2, base + 3) } else { (base + 6, base + 5) };
            board.castling |= 1 << idx;
            board.castle[idx] = CastleInfo {
                king, rook, king_to, rook_to,
                empty: (between(king, king_to) | bit(king_to) | between(rook, rook_to) | bit(rook_to))
                    & !bit(king) & !bit(rook),
                safe:  between(king, king_to) | bit(king) | bit(king_to),
            };
            board.castle_mask[rook as usize] &= !(1 << idx);
            board.castle_mask[king as usize] &= !(3 << (color as usize * 2));
        }

        board.ep_sq = str_to_sq(parts[3]);

        board.halfmove = parts.get(4).and_then(|s| s.parse().ok()).unwrap_or(0);
        board.fullmove = parts.get(5).and_then(|s| s.parse().ok()).unwrap_or(1);

        if board.side == Color::Black { board.hash ^= ZOBRIST.side; }
        for i in 0..4 {
            if board.castling & (1 << i) != 0 { board.hash ^= ZOBRIST.castle[i]; }
        }
        if let Some(ep) = board.ep_sq {
            board.hash ^= ZOBRIST.ep_file[(ep & 7) as usize];
        }

        board.refresh_nnue();

        Some(board)
    }

    pub fn to_fen(&self) -> String {

        let mut fen = String::new();
        for rank in (0..8).rev() {
            let mut empty = 0u8;
            for file in 0..8 {
                let sq = rank * 8 + file;
                if let Some((piece, color)) = self.sq_piece[sq as usize] {
                    if empty > 0 { fen.push((b'0' + empty) as char); empty = 0; }
                    fen.push(piece.to_char(color));
                } else {
                    empty += 1;
                }
            }
            if empty > 0 { fen.push((b'0' + empty) as char); }
            if rank > 0 { fen.push('/'); }
        }
        fen.push(' ');
        fen.push(if self.side == Color::White { 'w' } else { 'b' });
        fen.push(' ');
        if self.castling == 0 {
            fen.push('-');
        } else {

            for idx in 0..4 {
                if self.castling & (1 << idx) == 0 { continue; }
                let color = if idx < 2 { Color::White } else { Color::Black };
                let rook  = self.castle[idx].rook;
                let (base, file) = (rook & !7, rook & 7);
                let rooks = self.pieces[color as usize][Piece::Rook as usize] & (0xFFu64 << base);
                let shadowed = if idx % 2 == 0 { rooks >> rook > 1 } else { rooks & (bit(rook) - 1) != 0 };
                let c = if shadowed { (b'a' + file) as char } else if idx % 2 == 0 { 'k' } else { 'q' };
                fen.push(if color == Color::White { c.to_ascii_uppercase() } else { c });
            }
        }
        fen.push(' ');
        match self.ep_sq {
            Some(s) => fen.push_str(&sq_to_str(s)),
            None => fen.push('-'),
        }
        fen.push_str(&format!(" {} {}", self.halfmove, self.fullmove));
        fen
    }

    pub fn make_move(&mut self, mv: Move) {
        self.make_move_impl(mv, true);
    }

    #[cfg(test)]
    pub(crate) fn make_move_no_nnue(&mut self, mv: Move) {
        self.make_move_impl(mv, false);
    }

    fn make_move_impl(&mut self, mv: Move, update_nnue: bool) {
        let from  = mv.from();
        let to    = mv.to();
        let flags = mv.flags();
        let us    = self.side;
        let them  = us.flip();

        let is_castle = matches!(flags, MoveFlag::CastleKing | MoveFlag::CastleQueen);
        let irrev = IrrevState {

            captured: if is_castle { None } else { self.sq_piece[to as usize].map(|(p, _)| p) },
            ep_sq:    self.ep_sq,
            castling: self.castling,
            halfmove: self.halfmove,
            hash:     self.hash,
        };

        let (moving_piece, _) = self.sq_piece[from as usize].unwrap();

        let mut feat: [FeatDelta; 4] = [(Piece::Pawn, Color::White, 0, false); 4];
        let mut nfeat = 0usize;

        feat[nfeat] = (moving_piece, us, from, false); nfeat += 1;

        if moving_piece == Piece::Pawn || mv.is_capture() || mv.is_promotion() {
            self.halfmove = 0;
        } else {
            self.halfmove += 1;
        }

        if let Some(ep) = self.ep_sq {
            self.hash ^= ZOBRIST.ep_file[(ep & 7) as usize];
        }

        self.ep_sq = None;

        self.clear_piece(from);

        match flags {
            MoveFlag::Quiet | MoveFlag::Capture => {
                if let Some(cp) = irrev.captured {
                    feat[nfeat] = (cp, them, to, false); nfeat += 1;
                }
                self.clear_piece(to);
                self.set_piece(to, moving_piece, us);
                feat[nfeat] = (moving_piece, us, to, true); nfeat += 1;
            }
            MoveFlag::DoublePush => {
                self.set_piece(to, moving_piece, us);
                feat[nfeat] = (moving_piece, us, to, true); nfeat += 1;

                let ep = if us == Color::White { to - 8 } else { to + 8 };
                self.ep_sq = Some(ep);
                self.hash ^= ZOBRIST.ep_file[(ep & 7) as usize];
            }
            MoveFlag::CastleKing | MoveFlag::CastleQueen => {

                let c = self.castle[castle_index(us, flags)];
                self.clear_piece(c.rook);
                self.set_piece(to, Piece::King, us);
                feat[nfeat] = (Piece::King, us, to, true); nfeat += 1;
                feat[nfeat] = (Piece::Rook, us, c.rook, false); nfeat += 1;
                self.set_piece(c.rook_to, Piece::Rook, us);
                feat[nfeat] = (Piece::Rook, us, c.rook_to, true); nfeat += 1;
            }
            MoveFlag::EnPassant => {
                let cap_sq = if us == Color::White { to - 8 } else { to + 8 };
                feat[nfeat] = (Piece::Pawn, them, cap_sq, false); nfeat += 1;
                self.clear_piece(cap_sq);
                self.set_piece(to, Piece::Pawn, us);
                feat[nfeat] = (Piece::Pawn, us, to, true); nfeat += 1;
            }
            MoveFlag::PromKnight | MoveFlag::PromKnightCapture => {
                if let Some(cp) = irrev.captured { feat[nfeat] = (cp, them, to, false); nfeat += 1; }
                self.clear_piece(to); self.set_piece(to, Piece::Knight, us);
                feat[nfeat] = (Piece::Knight, us, to, true); nfeat += 1;
            }
            MoveFlag::PromBishop | MoveFlag::PromBishopCapture => {
                if let Some(cp) = irrev.captured { feat[nfeat] = (cp, them, to, false); nfeat += 1; }
                self.clear_piece(to); self.set_piece(to, Piece::Bishop, us);
                feat[nfeat] = (Piece::Bishop, us, to, true); nfeat += 1;
            }
            MoveFlag::PromRook | MoveFlag::PromRookCapture => {
                if let Some(cp) = irrev.captured { feat[nfeat] = (cp, them, to, false); nfeat += 1; }
                self.clear_piece(to); self.set_piece(to, Piece::Rook, us);
                feat[nfeat] = (Piece::Rook, us, to, true); nfeat += 1;
            }
            MoveFlag::PromQueen | MoveFlag::PromQueenCapture => {
                if let Some(cp) = irrev.captured { feat[nfeat] = (cp, them, to, false); nfeat += 1; }
                self.clear_piece(to); self.set_piece(to, Piece::Queen, us);
                feat[nfeat] = (Piece::Queen, us, to, true); nfeat += 1;
            }
        }

        if update_nnue && self.net.is_some() {
            let mut buckets = [self.nnue_stack.bucket(Color::White), self.nnue_stack.bucket(Color::Black)];
            let mut refresh = 0u8;
            if moving_piece == Piece::King {
                let bucket = crate::nnue::king_bucket(us, to);
                if bucket != buckets[us as usize] {
                    buckets[us as usize] = bucket;
                    refresh = 1 << us as u8;
                }
            }
            self.nnue_stack.push(&feat[..nfeat], refresh, buckets, &self.pieces);
        }

        let old_castling = self.castling;
        self.castling &= self.castle_mask[from as usize];
        self.castling &= self.castle_mask[to   as usize];

        let changed = old_castling ^ self.castling;
        for i in 0..4u8 {
            if changed & (1 << i) != 0 { self.hash ^= ZOBRIST.castle[i as usize]; }
        }

        self.hash ^= ZOBRIST.side;

        if us == Color::Black { self.fullmove += 1; }

        self.side = them;
        self.history.push((mv, irrev));
    }

    pub fn unmake_move(&mut self) {
        self.unmake_move_impl(true);
    }

    #[cfg(test)]
    pub(crate) fn unmake_move_no_nnue(&mut self) {
        self.unmake_move_impl(false);
    }

    fn unmake_move_impl(&mut self, update_nnue: bool) {
        let (mv, irrev) = match self.history.pop() {
            Some(x) => x,
            None    => return,
        };

        let from  = mv.from();
        let to    = mv.to();
        let flags = mv.flags();

        self.side = self.side.flip();
        let us    = self.side;

        if us == Color::Black { self.fullmove -= 1; }

        self.ep_sq    = irrev.ep_sq;
        self.castling = irrev.castling;
        self.halfmove = irrev.halfmove;

        let (moved_piece, _) = self.sq_piece[to as usize].unwrap();

        match flags {
            MoveFlag::Quiet | MoveFlag::Capture | MoveFlag::DoublePush => {
                self.clear_piece(to);

                if let Some(cap) = irrev.captured {
                    self.set_piece(to, cap, us.flip());
                }
                let restore = if flags == MoveFlag::DoublePush { Piece::Pawn } else { moved_piece };
                self.set_piece(from, restore, us);
            }
            MoveFlag::CastleKing | MoveFlag::CastleQueen => {

                let c = self.castle[castle_index(us, flags)];
                self.clear_piece(to);
                self.clear_piece(c.rook_to);
                self.set_piece(from, Piece::King, us);
                self.set_piece(c.rook, Piece::Rook, us);
            }
            MoveFlag::EnPassant => {
                self.clear_piece(to);
                self.set_piece(from, Piece::Pawn, us);
                let cap_sq = if us == Color::White { to - 8 } else { to + 8 };
                self.set_piece(cap_sq, Piece::Pawn, us.flip());
            }

            MoveFlag::PromKnight | MoveFlag::PromBishop | MoveFlag::PromRook | MoveFlag::PromQueen => {
                self.clear_piece(to);
                self.set_piece(from, Piece::Pawn, us);
            }
            MoveFlag::PromKnightCapture | MoveFlag::PromBishopCapture
            | MoveFlag::PromRookCapture | MoveFlag::PromQueenCapture => {
                self.clear_piece(to);
                if let Some(cap) = irrev.captured {
                    self.set_piece(to, cap, us.flip());
                }
                self.set_piece(from, Piece::Pawn, us);
            }
        }

        self.hash = irrev.hash;

        if update_nnue && self.net.is_some() && !self.nnue_stack.pop() {
            self.refresh_nnue();
        }
    }

    #[inline]
    pub fn is_attacked(&self, sq: u8, attacker: Color) -> bool {
        self.is_attacked_occ(sq, attacker, self.occupied())
    }

    pub fn attackers_to(&self, sq: u8, by: Color, occ: Bitboard) -> Bitboard {
        let c = by as usize;
        let target = bit(sq);

        let pawn_from = if by == Color::White {
            ((target >> 7) & NOT_FILE_A) | ((target >> 9) & NOT_FILE_H)
        } else {
            ((target << 7) & NOT_FILE_H) | ((target << 9) & NOT_FILE_A)
        };

        (pawn_from & self.pieces[c][Piece::Pawn as usize])
            | (KNIGHT_ATTACKS[sq as usize] & self.pieces[c][Piece::Knight as usize])
            | (KING_ATTACKS[sq as usize] & self.pieces[c][Piece::King as usize])
            | (MAGICS.bishop_attacks(sq, occ)
                & (self.pieces[c][Piece::Bishop as usize] | self.pieces[c][Piece::Queen as usize]))
            | (MAGICS.rook_attacks(sq, occ)
                & (self.pieces[c][Piece::Rook as usize] | self.pieces[c][Piece::Queen as usize]))
    }

    pub fn is_attacked_occ(&self, sq: u8, attacker: Color, occ: Bitboard) -> bool {
        let a = attacker as usize;

        let pawns = self.pieces[a][Piece::Pawn as usize];
        let pawn_attacks = if attacker == Color::White {
            ((pawns << 9) & NOT_FILE_A) | ((pawns << 7) & NOT_FILE_H)
        } else {
            ((pawns >> 7) & NOT_FILE_A) | ((pawns >> 9) & NOT_FILE_H)
        };
        if pawn_attacks & bit(sq) != 0 { return true; }

        if KNIGHT_ATTACKS[sq as usize] & self.pieces[a][Piece::Knight as usize] != 0 { return true; }

        if KING_ATTACKS[sq as usize] & self.pieces[a][Piece::King as usize] != 0 { return true; }

        let diag_attackers = self.pieces[a][Piece::Bishop as usize]
            | self.pieces[a][Piece::Queen as usize];
        if MAGICS.bishop_attacks(sq, occ) & diag_attackers != 0 { return true; }

        let orth_attackers = self.pieces[a][Piece::Rook as usize]
            | self.pieces[a][Piece::Queen as usize];
        if MAGICS.rook_attacks(sq, occ) & orth_attackers != 0 { return true; }

        false
    }

    pub fn king_sq(&self, color: Color) -> u8 {
        lsb(self.pieces[color as usize][Piece::King as usize])
    }

    pub fn attacks_by(&self, color: Color) -> Bitboard {
        let c   = color as usize;
        let occ = self.occupied();

        let pawns = self.pieces[c][Piece::Pawn as usize];
        let mut atk = if color == Color::White {
            ((pawns << 9) & NOT_FILE_A) | ((pawns << 7) & NOT_FILE_H)
        } else {
            ((pawns >> 7) & NOT_FILE_A) | ((pawns >> 9) & NOT_FILE_H)
        };

        let mut knights = self.pieces[c][Piece::Knight as usize];
        while knights != 0 {
            atk |= KNIGHT_ATTACKS[pop_lsb(&mut knights) as usize];
        }

        atk |= KING_ATTACKS[self.king_sq(color) as usize];

        let mut diag = self.pieces[c][Piece::Bishop as usize] | self.pieces[c][Piece::Queen as usize];
        while diag != 0 {
            atk |= MAGICS.bishop_attacks(pop_lsb(&mut diag), occ);
        }

        let mut orth = self.pieces[c][Piece::Rook as usize] | self.pieces[c][Piece::Queen as usize];
        while orth != 0 {
            atk |= MAGICS.rook_attacks(pop_lsb(&mut orth), occ);
        }

        atk
    }

    pub fn in_check(&self) -> bool {
        self.is_attacked(self.king_sq(self.side), self.side.flip())
    }

    pub fn print(&self) {
        println!("+---+---+---+---+---+---+---+---+");
        for rank in (0..8).rev() {
            print!("|");
            for file in 0..8 {
                let sq = rank * 8 + file;
                let s = match self.sq_piece[sq] {
                    Some((p, c)) => format!(" {} ", p.to_char(c)),
                    None         => "   ".to_string(),
                };
                print!("{}|", s);
            }
            println!(" {}", rank + 1);
            println!("+---+---+---+---+---+---+---+---+");
        }
        println!("  a   b   c   d   e   f   g   h");
        println!("Side: {:?}  EP: {:?}  Castle: {:04b}",
            self.side, self.ep_sq, self.castling);
    }

    pub fn make_null_move(&mut self) {

        let irrev = IrrevState {
            captured: None,
            ep_sq:    self.ep_sq,
            castling: self.castling,
            halfmove: self.halfmove,
            hash:     self.hash,
        };

        if let Some(ep) = self.ep_sq {
            self.hash ^= ZOBRIST.ep_file[(ep & 7) as usize];
        }
        self.ep_sq = None;

        self.hash ^= ZOBRIST.side;
        self.side = self.side.flip();
        self.halfmove += 1;
        if self.side == Color::White { self.fullmove += 1; }

        self.history.push((Move::NULL, irrev));
    }

    pub fn unmake_null_move(&mut self) {
        let (_, irrev) = match self.history.pop() {
            Some(x) => x,
            None    => return,
        };
        self.side     = self.side.flip();
        self.ep_sq    = irrev.ep_sq;
        self.castling = irrev.castling;
        self.halfmove = irrev.halfmove;
        self.hash     = irrev.hash;
        if self.side == Color::Black { self.fullmove -= 1; }
    }

    pub fn apply_moves(&mut self, move_strs: &[&str]) {
        for ms in move_strs {
            if let Some(mv) = self.find_uci_move(ms) {
                self.make_move(mv);
            }
        }
    }

    pub fn find_uci_move(&self, uci: &str) -> Option<Move> {
        use crate::movegen::MoveGen;
        let parsed = Move::from_uci(uci)?;
        let mut moves = crate::moves::MoveList::new();
        MoveGen::generate_all(self, &mut moves);

        moves.iter().copied().find(|mv| {
            mv.from() == parsed.from()
                && (mv.to() == parsed.to() || self.castle_rook(*mv) == Some(parsed.to()))
                && mv.promo_piece() == parsed.promo_piece()
        })
    }

    #[inline]
    pub(crate) fn castle_rook(&self, mv: Move) -> Option<u8> {
        let flags = mv.flags();
        if !matches!(flags, MoveFlag::CastleKing | MoveFlag::CastleQueen) { return None; }
        let us = if mv.from() < 8 { Color::White } else { Color::Black };
        Some(self.castle[castle_index(us, flags)].rook)
    }

    pub fn move_to_uci(&self, mv: Move) -> String {
        match self.castle_rook(mv) {
            Some(rook) if CHESS960.load(std::sync::atomic::Ordering::Relaxed) =>
                format!("{}{}", sq_to_str(mv.from()), sq_to_str(rook)),
            _ => mv.to_uci(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn uci_underpromotions_resolve_to_the_named_piece() {
        crate::nnue::load_embedded();
        for (s, piece) in [("n", Piece::Knight), ("b", Piece::Bishop), ("r", Piece::Rook), ("q", Piece::Queen)] {

            for uci in [format!("e7e8{s}"), format!("e7d8{s}")] {
                let mut b = Board::from_fen("3r3k/4P3/8/8/8/8/8/K7 w - - 0 1").unwrap();
                b.apply_moves(&[uci.as_str()]);
                assert_eq!(b.sq_piece[bit_index(&uci[2..4])], Some((piece, Color::White)), "{uci}");
                assert_eq!(b.side, Color::Black, "{uci} was not applied");
            }
        }
    }

    #[test]
    fn knight_underpromotion_from_a_real_game() {
        crate::nnue::load_embedded();
        let mut b = Board::from_fen("rnbqkbnr/pppp1pp1/4p3/7p/4P3/5P2/PPPPK1PP/RNBQ1BNR w kq - 0 4").unwrap();
        let moves = "e2e1 c7c5 d2d4 d7d5 e4d5 e6d5 f1b5 c8d7 b5d7 d8d7 b1c3 c5d4 d1d4 g8e7 g1e2 \
            b8c6 d4d3 e8c8 c1f4 d5d4 c3e4 e7g6 a2a3 g6f4 e2f4 d7c7 f4e2 c6e5 d3b3 d4d3 f3f4 d3e2 \
            f4e5 d8e8 e4g5 f8c5 g5f3 g7g5 f3g5 c7e5 g5f3 e5e7 b3c4 c8b8 c4f4 b8a8 g2g3 h8g8 f4d2 \
            c5b6 d2d5 e7e3 a1d1 e3f2 e1d2 e2e1n d2c1";
        let list: Vec<&str> = moves.split_whitespace().collect();
        b.apply_moves(&list);
        assert_eq!(b.history.len(), list.len(), "every move must apply");
        assert_eq!(b.sq_piece[sq::E1 as usize], Some((Piece::Knight, Color::Black)));
    }

    fn bit_index(s: &str) -> usize {
        str_to_sq(s).unwrap() as usize
    }
}

#[cfg(test)]
mod hash_tests {
    use super::*;

    #[test]
    fn hash_survives_make_unmake() {
        crate::nnue::load_embedded();
        for fen in [
            "r1bq1rk1/pp2bppp/2n1pn2/3p4/2PP4/2N1PN2/PP3PPP/R2QKB1R w KQ - 0 8",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbqkbnr/ppp1p1pp/8/3pPp2/8/8/PPPP1PPP/RNBQKBNR w KQkq f6 0 3",
        ] {
            let mut b = Board::from_fen(fen).unwrap();
            let root = b.hash;
            let mut moves = crate::moves::MoveList::new();
            crate::movegen::MoveGen::generate_all(&b, &mut moves);
            for &mv in moves.iter() {
                b.make_move(mv);
                let want = Board::from_fen(&b.to_fen()).unwrap().hash;
                assert_eq!(b.hash, want, "{fen}: hash after {} differs from its FEN's", mv.to_uci());
                b.unmake_move();
                assert_eq!(b.hash, root, "{fen}: hash not restored after unmaking {}", mv.to_uci());
            }
        }
    }
}
