use crate::board::{Board, Color, Piece};
use crate::fbk;
use crate::moves::Move;

pub fn fbk_position(board: &Board) -> fbk::Position {
    let mut squares = [None; 64];
    for (sq, cell) in board.sq_piece.iter().enumerate() {
        squares[sq] = cell.map(|(piece, color)| {
            let color = match color {
                Color::White => fbk::Color::White,
                Color::Black => fbk::Color::Black,
            };
            let piece = match piece {
                Piece::Pawn => fbk::Piece::Pawn,
                Piece::Knight => fbk::Piece::Knight,
                Piece::Bishop => fbk::Piece::Bishop,
                Piece::Rook => fbk::Piece::Rook,
                Piece::Queen => fbk::Piece::Queen,
                Piece::King => fbk::Piece::King,
            };
            (color, piece)
        });
    }
    let mut castling_rooks = 0u64;
    for (idx, info) in board.castle.iter().enumerate() {
        if board.castling & (1 << idx) != 0 {
            castling_rooks |= 1u64 << info.rook;
        }
    }
    fbk::Position {
        board: squares,
        black_to_move: board.side == Color::Black,
        castling_rooks,

        ep_square: board.ep_sq,
    }
}

pub fn fbk_pick(book: &fbk::Book, board: &Board, rand: u64) -> Option<Move> {
    let key = fbk_position(board).key();
    let mv = book.choose(key, &fbk::Policy::default(), rand)?;
    board.find_uci_move(&mv.to_string())
}

pub fn fbk_moves(book: &fbk::Book, board: &Board) -> Vec<(Move, u32)> {
    book.probe(fbk_position(board).key())
        .into_iter()
        .filter_map(|e| {
            board
                .find_uci_move(&e.mv.to_string())
                .map(|m| (m, e.games()))
        })
        .collect()
}

pub struct Rng(u64);

impl Rng {
    pub fn new(seed: u64) -> Rng {
        Rng(seed | 1)
    }

    pub fn from_time() -> Rng {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        Rng::new(nanos ^ 0x9E37_79B9_7F4A_7C15)
    }

    pub fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fbk_keys_match_reference() {
        let mut b = Board::from_fen("1r4kr/pppppppp/8/8/8/8/PPPPPPPP/1R4KR w HBhb - 0 1").unwrap();
        assert_eq!(fbk_position(&b).key(), 0xa1aa_db7b_111f_7fab);
        b.apply_moves(&["e2e4", "d7d5", "e4e5", "f7f5"]);
        assert_eq!(fbk_position(&b).key(), 0x847b_ea0a_e4d4_29e6, "ep f6");
        b.apply_moves(&["e5f6", "g8h8"]);
        assert_eq!(fbk_position(&b).key(), 0x6d6d_25f0_1cff_6489, "black O-O");
        b.apply_moves(&["g1b1"]);
        assert_eq!(fbk_position(&b).key(), 0xd428_c5dc_72ec_f425, "white O-O-O");

        assert_eq!(
            fbk_position(&b).key(),
            fbk::key_from_fen(&b.to_fen()).unwrap()
        );
        assert_eq!(
            fbk_position(&Board::start_pos()).key(),
            0xa49d_4ce9_1cd6_b7ce,
            "standard start"
        );
    }

    fn fbk_book(entries: &[(u64, u16, u32)]) -> fbk::Book {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"FBKBOOK\0");
        bytes.extend_from_slice(&1u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&32u16.to_le_bytes());
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes.extend_from_slice(&(entries.len() as u64).to_le_bytes());
        bytes.extend_from_slice(&0u64.to_le_bytes());
        bytes.extend_from_slice(&64u64.to_le_bytes());
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.resize(64, 0);
        for &(key, mv, games) in entries {
            bytes.extend_from_slice(&key.to_le_bytes());
            bytes.extend_from_slice(&mv.to_le_bytes());
            bytes.extend_from_slice(&0u16.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&games.to_le_bytes());
            bytes.extend_from_slice(&0u32.to_le_bytes());
            bytes.extend_from_slice(&i16::MIN.to_le_bytes());
            bytes.extend_from_slice(&[0u8; 6]);
        }
        fbk::Book::from_bytes(bytes).unwrap()
    }

    #[test]
    fn fbk_pick_resolves_chess960_castling() {
        let b = Board::from_fen("1r4kr/pppppppp/8/8/8/8/PPPPPPPP/1R4KR w HBhb - 0 1").unwrap();
        let key = fbk_position(&b).key();

        let book = fbk_book(&[(key, (6 << 6) | 7, 3), (key, (6 << 6) | 1, 1)]);
        let mut seen = [false; 2];
        for r in 0..4 {
            let mv = fbk_pick(&book, &b, r).expect("book move");
            match b.castle_rook(mv) {
                Some(7) => seen[0] = true,
                Some(1) => seen[1] = true,
                other => panic!("not castling: {other:?}"),
            }
        }
        assert_eq!(seen, [true, true]);

        assert_eq!(fbk_pick(&book, &Board::start_pos(), 0), None);
    }

    #[test]
    fn fbk_pick_resolves_standard_castling() {
        let b = Board::from_fen("r3k2r/8/8/8/8/8/8/R3K2R w KQkq - 0 1").unwrap();
        let book = fbk_book(&[(fbk_position(&b).key(), (4 << 6) | 7, 1)]);
        let mv = fbk_pick(&book, &b, 0).unwrap();
        assert_eq!(mv.to_uci(), "e1g1");
        assert_eq!(b.castle_rook(mv), Some(7));
    }

    #[test]
    fn fbk_moves_lists_legal_moves_in_order() {
        let b = Board::start_pos();
        let key = fbk_position(&b).key();

        let e4 = (12 << 6) | 28;
        let d4 = (11 << 6) | 27;
        let bad = (12 << 6) | 36;
        let book = fbk_book(&[(key, e4, 5), (key, d4, 2), (key, bad, 1)]);
        let moves: Vec<(String, u32)> = fbk_moves(&book, &b)
            .into_iter()
            .map(|(m, g)| (m.to_uci(), g))
            .collect();
        assert_eq!(
            moves,
            vec![("e2e4".to_string(), 5), ("d2d4".to_string(), 2)]
        );
    }
}
