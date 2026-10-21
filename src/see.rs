

use crate::bitboard::{bit, Bitboard, KING_ATTACKS, KNIGHT_ATTACKS, MAGICS};
use crate::board::{Board, Color, Piece};
use crate::moves::{Move, MoveFlag};

pub const SEE_VALUE: [i32; 6] = [100, 320, 330, 500, 900, 20_000];

pub fn see(board: &Board, mv: Move) -> i32 {

    if matches!(mv.flags(), MoveFlag::CastleKing | MoveFlag::CastleQueen) {
        return 0;
    }

    let from_sq = mv.from();
    let to_sq   = mv.to();
    let Some((mover, _)) = board.sq_piece[from_sq as usize] else { return 0 };

    let mut occ = board.occupied() & !bit(from_sq);

    let captured = if mv.flags() == MoveFlag::EnPassant {
        let cap_sq = if board.side == Color::White { to_sq - 8 } else { to_sq + 8 };
        occ &= !bit(cap_sq);
        SEE_VALUE[Piece::Pawn as usize]
    } else {
        board.sq_piece[to_sq as usize].map_or(0, |(p, _)| SEE_VALUE[p as usize])
    };

    let (landed_val, promo_gain) = match mv.promo_piece() {
        Some(p) => (SEE_VALUE[p as usize], SEE_VALUE[p as usize] - SEE_VALUE[Piece::Pawn as usize]),
        None    => (SEE_VALUE[mover as usize], 0),
    };

    captured + promo_gain - see_min(board, to_sq, board.side.flip(), occ, landed_val)
}

fn see_min(board: &Board, sq: u8, side: Color, occ: Bitboard, last_val: i32) -> i32 {
    let (attacker_sq, attacker_val) = match least_valuable_attacker(board, sq, side, occ) {
        Some(x) => x,
        None    => return 0,
    };

    let new_occ = occ & !bit(attacker_sq);

    let recapture = last_val - see_min(board, sq, side.flip(), new_occ, attacker_val);
    recapture.max(0)
}

fn least_valuable_attacker(
    board: &Board,
    sq:    u8,
    side:  Color,
    occ:   Bitboard,
) -> Option<(u8, i32)> {
    let s = side as usize;

    for &piece in &[Piece::Pawn, Piece::Knight, Piece::Bishop,
                    Piece::Rook, Piece::Queen,  Piece::King]
    {
        let attackers = piece_attacks_to(sq, piece, side, occ)
            & board.pieces[s][piece as usize]
            & occ;

        if attackers != 0 {
            let attacker_sq = attackers.trailing_zeros() as u8;
            return Some((attacker_sq, SEE_VALUE[piece as usize]));
        }
    }
    None
}

fn piece_attacks_to(sq: u8, piece: Piece, side: Color, occ: Bitboard) -> Bitboard {
    use crate::bitboard::{NOT_FILE_A, NOT_FILE_H};

    match piece {
        Piece::Pawn => {

            if side == Color::White {

                let sq_bb = bit(sq);
                ((sq_bb >> 7) & NOT_FILE_A) | ((sq_bb >> 9) & NOT_FILE_H)
            } else {
                let sq_bb = bit(sq);
                ((sq_bb << 7) & NOT_FILE_H) | ((sq_bb << 9) & NOT_FILE_A)
            }
        }
        Piece::Knight => KNIGHT_ATTACKS[sq as usize],
        Piece::Bishop => MAGICS.bishop_attacks(sq, occ),
        Piece::Rook   => MAGICS.rook_attacks(sq, occ),
        Piece::Queen  => MAGICS.queen_attacks(sq, occ),
        Piece::King   => KING_ATTACKS[sq as usize],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn see_uci(fen: &str, uci: &str) -> i32 {
        crate::nnue::load_embedded();
        let b = Board::from_fen(fen).unwrap();
        let mv = b.find_uci_move(uci).expect("legal move");
        see(&b, mv)
    }

    #[test]
    fn quiet_move_onto_attacked_square_loses_the_piece() {

        assert_eq!(see_uci("4k3/8/3p4/8/8/5N2/8/4K3 w - - 0 1", "f3e5"), -SEE_VALUE[1]);
    }

    #[test]
    fn safe_quiet_move_is_neutral() {
        assert_eq!(see_uci("4k3/8/8/8/8/5N2/8/4K3 w - - 0 1", "f3e5"), 0);
    }

    #[test]
    fn defended_pawn_capture_by_queen_loses() {

        assert_eq!(
            see_uci("4k3/2p5/3p4/8/8/8/8/3QK3 w - - 0 1", "d1d6"),
            SEE_VALUE[0] - SEE_VALUE[4],
        );
    }

    #[test]
    fn en_passant_counts_the_captured_pawn() {

        assert_eq!(see_uci("4k3/8/8/3pP3/8/8/8/4K3 w - d6 0 1", "e5d6"), SEE_VALUE[0]);
    }

    #[test]
    fn promotion_gain_is_included() {

        assert_eq!(see_uci("7k/P7/8/8/8/8/8/4K3 w - - 0 1", "a7a8q"), SEE_VALUE[4] - SEE_VALUE[0]);
    }

    #[test]
    fn castling_is_neutral() {
        assert_eq!(see_uci("4k3/8/8/8/8/8/8/4K2R w K - 0 1", "e1g1"), 0);
    }
}
