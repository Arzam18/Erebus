use crate::bitboard::*;
use crate::board::{Board, Color, Piece};
use crate::moves::{Move, MoveFlag, MoveList};

pub struct MoveGen;

impl MoveGen {
    pub fn generate_all(board: &Board, list: &mut MoveList) {
        Self::generate_pseudo_into(board, list);
        Self::retain_legal(board, list);
    }

    pub fn generate_captures_legal(board: &Board, list: &mut MoveList) {
        Self::generate_captures_pseudo_into(board, list);
        Self::retain_legal(board, list);
    }

    fn retain_legal(board: &Board, list: &mut MoveList) {
        let ctx = LegalCtx::new(board);
        let n = list.len();
        let mut w = 0;
        for r in 0..n {
            let mv = list[r];
            if ctx.is_legal(board, mv) {
                list.swap(w, r);
                w += 1;
            }
        }
        list.truncate(w);
    }

    fn generate_pseudo_into(board: &Board, moves: &mut MoveList) {
        let us = board.side;
        let them = us.flip();
        let occ = board.occupied();
        let ours = board.color_bb[us as usize];
        let theirs = board.color_bb[them as usize];

        Self::gen_pawns(board, us, occ, theirs, moves);
        Self::gen_knights(board, us, ours, theirs, moves);
        Self::gen_bishops(board, us, occ, ours, theirs, moves);
        Self::gen_rooks(board, us, occ, ours, theirs, moves);
        Self::gen_queens(board, us, occ, ours, theirs, moves);
        Self::gen_king(board, us, ours, theirs, moves);
        Self::gen_castles(board, us, occ, moves);
    }

    fn generate_captures_pseudo_into(board: &Board, moves: &mut MoveList) {
        let us = board.side;
        let them = us.flip();
        let occ = board.occupied();
        let theirs = board.color_bb[them as usize];

        Self::gen_pawn_captures(board, us, occ, theirs, moves);
        Self::gen_knight_captures(board, us, theirs, moves);
        Self::gen_bishop_captures(board, us, occ, theirs, moves);
        Self::gen_rook_captures(board, us, occ, theirs, moves);
        Self::gen_queen_captures(board, us, occ, theirs, moves);
        Self::gen_king_captures(board, us, theirs, moves);
    }

    fn gen_pawn_captures(
        board: &Board,
        us: Color,
        occ: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let pawns = board.piece_bb(us, Piece::Pawn);

        if us == Color::White {
            let cap_e = (pawns << 9) & NOT_FILE_A & theirs;
            let cap_w = (pawns << 7) & NOT_FILE_H & theirs;
            Self::add_pawn_moves(cap_e & !RANK_8, 9, MoveFlag::Capture, false, moves);
            Self::add_pawn_moves(cap_w & !RANK_8, 7, MoveFlag::Capture, false, moves);
            Self::add_promotions(((pawns << 8) & !occ) & RANK_8, 8, false, false, moves);
            Self::add_promotions(cap_e & RANK_8, 9, true, false, moves);
            Self::add_promotions(cap_w & RANK_8, 7, true, false, moves);
        } else {
            let cap_e = (pawns >> 7) & NOT_FILE_A & theirs;
            let cap_w = (pawns >> 9) & NOT_FILE_H & theirs;
            Self::add_pawn_moves(cap_e & !RANK_1, 7, MoveFlag::Capture, true, moves);
            Self::add_pawn_moves(cap_w & !RANK_1, 9, MoveFlag::Capture, true, moves);
            Self::add_promotions(((pawns >> 8) & !occ) & RANK_1, 8, false, true, moves);
            Self::add_promotions(cap_e & RANK_1, 7, true, true, moves);
            Self::add_promotions(cap_w & RANK_1, 9, true, true, moves);
        }
    }

    fn gen_pawns(board: &Board, us: Color, occ: Bitboard, theirs: Bitboard, moves: &mut MoveList) {
        let pawns = board.piece_bb(us, Piece::Pawn);
        let empty = !occ;

        if us == Color::White {
            let push1 = (pawns << 8) & empty;
            let push2 = ((push1 & RANK_3) << 8) & empty;
            let cap_e = (pawns << 9) & NOT_FILE_A & theirs;
            let cap_w = (pawns << 7) & NOT_FILE_H & theirs;

            Self::add_pawn_moves(push1 & !RANK_8, 8, MoveFlag::Quiet, false, moves);
            Self::add_pawn_moves(push2, 16, MoveFlag::DoublePush, false, moves);
            Self::add_pawn_moves(cap_e & !RANK_8, 9, MoveFlag::Capture, false, moves);
            Self::add_pawn_moves(cap_w & !RANK_8, 7, MoveFlag::Capture, false, moves);
            Self::add_promotions(push1 & RANK_8, 8, false, false, moves);
            Self::add_promotions(cap_e & RANK_8, 9, true, false, moves);
            Self::add_promotions(cap_w & RANK_8, 7, true, false, moves);
        } else {
            let push1 = (pawns >> 8) & empty;
            let push2 = ((push1 & RANK_6) >> 8) & empty;
            let cap_e = (pawns >> 7) & NOT_FILE_A & theirs;
            let cap_w = (pawns >> 9) & NOT_FILE_H & theirs;

            Self::add_pawn_moves(push1 & !RANK_1, 8, MoveFlag::Quiet, true, moves);
            Self::add_pawn_moves(push2, 16, MoveFlag::DoublePush, true, moves);
            Self::add_pawn_moves(cap_e & !RANK_1, 7, MoveFlag::Capture, true, moves);
            Self::add_pawn_moves(cap_w & !RANK_1, 9, MoveFlag::Capture, true, moves);
            Self::add_promotions(push1 & RANK_1, 8, false, true, moves);
            Self::add_promotions(cap_e & RANK_1, 7, true, true, moves);
            Self::add_promotions(cap_w & RANK_1, 9, true, true, moves);
        }
        Self::gen_en_passant(board, us, moves);
    }

    fn gen_en_passant(board: &Board, us: Color, moves: &mut MoveList) {
        let Some(ep) = board.ep_sq else { return };
        let pawns = board.piece_bb(us, Piece::Pawn);
        let ep_bb = bit(ep);
        if us == Color::White {
            if (pawns << 9) & NOT_FILE_A & ep_bb != 0 {
                moves.push(Move::new(ep - 9, ep, MoveFlag::EnPassant));
            }
            if (pawns << 7) & NOT_FILE_H & ep_bb != 0 {
                moves.push(Move::new(ep - 7, ep, MoveFlag::EnPassant));
            }
        } else {
            if (pawns >> 7) & NOT_FILE_A & ep_bb != 0 {
                moves.push(Move::new(ep + 7, ep, MoveFlag::EnPassant));
            }
            if (pawns >> 9) & NOT_FILE_H & ep_bb != 0 {
                moves.push(Move::new(ep + 9, ep, MoveFlag::EnPassant));
            }
        }
    }

    fn add_pawn_moves(
        mut targets: Bitboard,
        delta: u8,
        flag: MoveFlag,
        black: bool,
        moves: &mut MoveList,
    ) {
        while targets != 0 {
            let to = pop_lsb(&mut targets);
            let from = if black { to + delta } else { to - delta };
            moves.push(Move::new(from, to, flag));
        }
    }

    fn add_promotions(
        mut targets: Bitboard,
        delta: u8,
        capture: bool,
        black: bool,
        moves: &mut MoveList,
    ) {
        while targets != 0 {
            let to = pop_lsb(&mut targets);
            let from = if black { to + delta } else { to - delta };
            let (qf, rf, bf, nf) = if capture {
                (
                    MoveFlag::PromQueenCapture,
                    MoveFlag::PromRookCapture,
                    MoveFlag::PromBishopCapture,
                    MoveFlag::PromKnightCapture,
                )
            } else {
                (
                    MoveFlag::PromQueen,
                    MoveFlag::PromRook,
                    MoveFlag::PromBishop,
                    MoveFlag::PromKnight,
                )
            };
            moves.push(Move::new(from, to, qf));
            moves.push(Move::new(from, to, rf));
            moves.push(Move::new(from, to, bf));
            moves.push(Move::new(from, to, nf));
        }
    }

    fn gen_knight_captures(board: &Board, us: Color, theirs: Bitboard, moves: &mut MoveList) {
        let mut knights = board.piece_bb(us, Piece::Knight);
        while knights != 0 {
            let from = pop_lsb(&mut knights);
            let mut attacks = KNIGHT_ATTACKS[from as usize] & theirs;
            while attacks != 0 {
                let to = pop_lsb(&mut attacks);
                moves.push(Move::new(from, to, MoveFlag::Capture));
            }
        }
    }

    fn gen_knights(
        board: &Board,
        us: Color,
        ours: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut knights = board.piece_bb(us, Piece::Knight);
        while knights != 0 {
            let from = pop_lsb(&mut knights);
            let mut attacks = KNIGHT_ATTACKS[from as usize] & !ours;
            while attacks != 0 {
                let to = pop_lsb(&mut attacks);
                let flag = if bit(to) & theirs != 0 {
                    MoveFlag::Capture
                } else {
                    MoveFlag::Quiet
                };
                moves.push(Move::new(from, to, flag));
            }
        }
    }

    fn gen_bishop_captures(
        board: &Board,
        us: Color,
        occ: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut bishops = board.piece_bb(us, Piece::Bishop);
        while bishops != 0 {
            let from = pop_lsb(&mut bishops);
            let attacks = MAGICS.bishop_attacks(from, occ) & theirs;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn gen_bishops(
        board: &Board,
        us: Color,
        occ: Bitboard,
        ours: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut bishops = board.piece_bb(us, Piece::Bishop);
        while bishops != 0 {
            let from = pop_lsb(&mut bishops);
            let attacks = MAGICS.bishop_attacks(from, occ) & !ours;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn gen_rook_captures(
        board: &Board,
        us: Color,
        occ: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut rooks = board.piece_bb(us, Piece::Rook);
        while rooks != 0 {
            let from = pop_lsb(&mut rooks);
            let attacks = MAGICS.rook_attacks(from, occ) & theirs;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn gen_rooks(
        board: &Board,
        us: Color,
        occ: Bitboard,
        ours: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut rooks = board.piece_bb(us, Piece::Rook);
        while rooks != 0 {
            let from = pop_lsb(&mut rooks);
            let attacks = MAGICS.rook_attacks(from, occ) & !ours;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn gen_queen_captures(
        board: &Board,
        us: Color,
        occ: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut queens = board.piece_bb(us, Piece::Queen);
        while queens != 0 {
            let from = pop_lsb(&mut queens);
            let attacks = MAGICS.queen_attacks(from, occ) & theirs;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn gen_queens(
        board: &Board,
        us: Color,
        occ: Bitboard,
        ours: Bitboard,
        theirs: Bitboard,
        moves: &mut MoveList,
    ) {
        let mut queens = board.piece_bb(us, Piece::Queen);
        while queens != 0 {
            let from = pop_lsb(&mut queens);
            let attacks = MAGICS.queen_attacks(from, occ) & !ours;
            Self::add_sliding(from, attacks, theirs, moves);
        }
    }

    fn add_sliding(from: u8, mut attacks: Bitboard, theirs: Bitboard, moves: &mut MoveList) {
        while attacks != 0 {
            let to = pop_lsb(&mut attacks);
            let flag = if bit(to) & theirs != 0 {
                MoveFlag::Capture
            } else {
                MoveFlag::Quiet
            };
            moves.push(Move::new(from, to, flag));
        }
    }

    fn gen_king_captures(board: &Board, us: Color, theirs: Bitboard, moves: &mut MoveList) {
        let from = board.king_sq(us);
        if from < 64 {
            let mut attacks = KING_ATTACKS[from as usize] & theirs;
            while attacks != 0 {
                let to = pop_lsb(&mut attacks);
                moves.push(Move::new(from, to, MoveFlag::Capture));
            }
        }
    }

    fn gen_king(board: &Board, us: Color, ours: Bitboard, theirs: Bitboard, moves: &mut MoveList) {
        let from = board.king_sq(us);
        if from >= 64 {
            return;
        }
        let mut attacks = KING_ATTACKS[from as usize] & !ours;
        while attacks != 0 {
            let to = pop_lsb(&mut attacks);
            let flag = if bit(to) & theirs != 0 {
                MoveFlag::Capture
            } else {
                MoveFlag::Quiet
            };
            moves.push(Move::new(from, to, flag));
        }
    }

    fn gen_castles(board: &Board, us: Color, occ: Bitboard, moves: &mut MoveList) {
        let them = us.flip();
        let base = us as usize * 2;
        for (idx, flag) in [
            (base, MoveFlag::CastleKing),
            (base + 1, MoveFlag::CastleQueen),
        ] {
            if board.castling & (1 << idx) == 0 {
                continue;
            }
            let c = &board.castle[idx];
            if occ & c.empty != 0 || board.sq_piece[c.rook as usize] != Some((Piece::Rook, us)) {
                continue;
            }

            let occ_no_rook = occ ^ bit(c.rook);
            let mut safe = c.safe;
            let mut ok = true;
            while safe != 0 {
                if board.is_attacked_occ(pop_lsb(&mut safe), them, occ_no_rook) {
                    ok = false;
                    break;
                }
            }
            if ok {
                moves.push(Move::new(c.king, c.king_to, flag));
            }
        }
    }

    pub fn perft(board: &mut Board, depth: u32) -> u64 {
        let mut moves = MoveList::new();
        Self::generate_all(board, &mut moves);
        if depth <= 1 {
            return moves.len() as u64;
        }
        let mut nodes = 0u64;
        for i in 0..moves.len() {
            let mv = moves[i];
            board.make_move(mv);
            nodes += Self::perft(board, depth - 1);
            board.unmake_move();
        }
        nodes
    }

    pub fn perft_divide(board: &mut Board, depth: u32) -> Vec<(String, u64)> {
        let mut moves = MoveList::new();
        Self::generate_all(board, &mut moves);
        let mut result = Vec::new();
        for i in 0..moves.len() {
            let mv = moves[i];
            board.make_move(mv);
            let nodes = if depth <= 1 {
                1
            } else {
                Self::perft(board, depth - 1)
            };
            board.unmake_move();
            result.push((board.move_to_uci(mv), nodes));
        }
        result
    }
}

struct LegalCtx {
    us: Color,
    ksq: u8,
    occ: Bitboard,
    num_checkers: u32,

    check_block: Bitboard,

    pinned: Bitboard,
}

impl LegalCtx {
    fn new(board: &Board) -> Self {
        let us = board.side;
        let them = us.flip();
        let ksq = board.king_sq(us);
        let occ = board.occupied();

        let checkers = board.attackers_to(ksq, them, occ);
        let num_checkers = checkers.count_ones();

        let check_block = if num_checkers == 1 {
            let csq = lsb(checkers);
            checkers | between(ksq, csq)
        } else {
            !0u64
        };

        let tc = them as usize;
        let rq = board.pieces[tc][Piece::Rook as usize] | board.pieces[tc][Piece::Queen as usize];
        let bq = board.pieces[tc][Piece::Bishop as usize] | board.pieces[tc][Piece::Queen as usize];
        let mut snipers = (MAGICS.rook_attacks(ksq, 0) & rq) | (MAGICS.bishop_attacks(ksq, 0) & bq);
        let own = board.color_bb[us as usize];
        let mut pinned = 0u64;
        while snipers != 0 {
            let s = pop_lsb(&mut snipers);
            let b = between(ksq, s) & occ;
            if b.count_ones() == 1 && (b & own) != 0 {
                pinned |= b;
            }
        }

        LegalCtx {
            us,
            ksq,
            occ,
            num_checkers,
            check_block,
            pinned,
        }
    }

    fn is_legal(&self, board: &Board, mv: Move) -> bool {
        let from = mv.from();
        let to = mv.to();
        let flags = mv.flags();
        let them = self.us.flip();

        if matches!(flags, MoveFlag::CastleKing | MoveFlag::CastleQueen) {
            return true;
        }

        if from == self.ksq {
            let occ_after = (self.occ ^ bit(from)) | bit(to);
            return board.attackers_to(to, them, occ_after) == 0;
        }

        if self.num_checkers >= 2 {
            return false;
        }

        if flags == MoveFlag::EnPassant {
            let cap_sq = if self.us == Color::White {
                to - 8
            } else {
                to + 8
            };
            let occ_after = (self.occ ^ bit(from) ^ bit(cap_sq)) | bit(to);
            let tc = them as usize;
            let rq =
                board.pieces[tc][Piece::Rook as usize] | board.pieces[tc][Piece::Queen as usize];
            let bq =
                board.pieces[tc][Piece::Bishop as usize] | board.pieces[tc][Piece::Queen as usize];
            let ktgt = bit(self.ksq);
            let pawn_from = if them == Color::White {
                ((ktgt >> 7) & NOT_FILE_A) | ((ktgt >> 9) & NOT_FILE_H)
            } else {
                ((ktgt << 7) & NOT_FILE_H) | ((ktgt << 9) & NOT_FILE_A)
            };
            let attacked = MAGICS.rook_attacks(self.ksq, occ_after) & rq != 0
                || MAGICS.bishop_attacks(self.ksq, occ_after) & bq != 0
                || KNIGHT_ATTACKS[self.ksq as usize] & board.pieces[tc][Piece::Knight as usize]
                    != 0
                || KING_ATTACKS[self.ksq as usize] & board.pieces[tc][Piece::King as usize] != 0
                || pawn_from & board.pieces[tc][Piece::Pawn as usize] & !bit(cap_sq) != 0;
            return !attacked;
        }

        if self.num_checkers == 1 && (bit(to) & self.check_block) == 0 {
            return false;
        }

        if (bit(from) & self.pinned) != 0 && (bit(to) & line(self.ksq, from)) == 0 {
            return false;
        }

        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::board::Board;
    use crate::moves::MoveFlag;

    fn legal_slow(board: &mut Board) -> Vec<Move> {
        let mut pseudo = MoveList::new();
        MoveGen::generate_pseudo_into(board, &mut pseudo);
        let mut out = Vec::new();
        for i in 0..pseudo.len() {
            let mv = pseudo[i];
            board.make_move_no_nnue(mv);
            let legal = !board.is_attacked(board.king_sq(board.side.flip()), board.side);
            board.unmake_move_no_nnue();
            if legal {
                out.push(mv);
            }
        }
        out
    }

    fn sorted_uci(mut v: Vec<Move>) -> Vec<String> {
        let mut s: Vec<String> = v.drain(..).map(|m| m.to_uci()).collect();
        s.sort();
        s
    }

    fn assert_matches(fen: &str) {
        let mut b = Board::from_fen(fen).expect("fen");
        let fast = {
            let mut l = MoveList::new();
            MoveGen::generate_all(&b, &mut l);
            (0..l.len()).map(|i| l[i]).collect::<Vec<_>>()
        };
        let slow = legal_slow(&mut b);
        assert_eq!(sorted_uci(fast), sorted_uci(slow), "mismatch on {fen}");
    }

    fn assert_order_matches(board: &mut Board, depth: u32) {
        let fast = {
            let mut l = MoveList::new();
            MoveGen::generate_all(board, &mut l);
            (0..l.len()).map(|i| l[i].to_uci()).collect::<Vec<_>>()
        };
        let slow = legal_slow(board)
            .iter()
            .map(|m| m.to_uci())
            .collect::<Vec<_>>();
        assert_eq!(fast, slow, "order mismatch at {}", board.to_fen());
        if depth == 0 {
            return;
        }
        let mut l = MoveList::new();
        MoveGen::generate_all(board, &mut l);
        for i in 0..l.len() {
            let mv = l[i];
            board.make_move(mv);
            assert_order_matches(board, depth - 1);
            board.unmake_move();
        }
    }

    #[test]
    fn order_matches_reference_deep() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
        ] {
            let mut b = Board::from_fen(fen).unwrap();
            assert_order_matches(&mut b, 3);
        }
    }

    #[test]
    fn legal_matches_reference() {
        for fen in [
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1",
            "r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1",
            "8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1",
            "r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1",
            "rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8",
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
            "8/8/8/K2pP2r/8/8/8/7k w - d6 0 1",
            "rnb1kbnr/pppp1ppp/8/4p3/6Pq/5P2/PPPPP2P/RNBQKBNR w KQkq - 1 3",
            "4k3/8/8/8/8/8/3n4/4K1r1 w - - 0 1",
        ] {
            assert_matches(fen);
        }
    }

    fn perft(b: &mut Board, d: u32) -> u64 {
        MoveGen::perft(b, d)
    }

    #[test]
    fn perft_startpos() {
        let mut b = Board::start_pos();
        assert_eq!(perft(&mut b, 4), 197281);
    }

    #[test]
    fn perft_kiwipete() {
        let mut b =
            Board::from_fen("r3k2r/p1ppqpb1/bn2pnp1/3PN3/1p2P3/2N2Q1p/PPPBBPPP/R3K2R w KQkq - 0 1")
                .unwrap();
        assert_eq!(perft(&mut b, 3), 97862);
    }

    #[test]
    fn perft_pos3() {
        let mut b = Board::from_fen("8/2p5/3p4/KP5r/1R3p1k/8/4P1P1/8 w - - 0 1").unwrap();
        assert_eq!(perft(&mut b, 5), 674624);
    }

    #[test]
    fn perft_pos4() {
        let mut b =
            Board::from_fen("r3k2r/Pppp1ppp/1b3nbN/nP6/BBP1P3/q4N2/Pp1P2PP/R2Q1RK1 w kq - 0 1")
                .unwrap();
        assert_eq!(perft(&mut b, 4), 422333);
    }

    #[test]
    fn perft_pos5() {
        let mut b =
            Board::from_fen("rnbq1k1r/pp1Pbppp/2p5/8/2B5/8/PPP1NnPP/RNBQK2R w KQ - 1 8").unwrap();
        assert_eq!(perft(&mut b, 3), 62379);
    }

    #[test]
    fn perft_pos6() {
        let mut b = Board::from_fen(
            "r4rk1/1pp1qppp/p1np1n2/2b1p1B1/2B1P1b1/P1NP1N2/1PP1QPPP/R4RK1 w - - 0 10",
        )
        .unwrap();
        assert_eq!(perft(&mut b, 4), 3894594);
    }

    fn chess960_fen(n: usize) -> String {
        let mut rank = [' '; 8];
        let mut n = n;
        rank[(n % 4) * 2 + 1] = 'b';
        n /= 4;
        rank[(n % 4) * 2] = 'b';
        n /= 4;
        let put = |rank: &mut [char; 8], k: usize, c: char| {
            let f = (0..8).filter(|&f| rank[f] == ' ').nth(k).unwrap();
            rank[f] = c;
        };
        put(&mut rank, n % 6, 'q');
        n /= 6;
        let (a, b) = [
            (0, 1),
            (0, 2),
            (0, 3),
            (0, 4),
            (1, 2),
            (1, 3),
            (1, 4),
            (2, 3),
            (2, 4),
            (3, 4),
        ][n];
        put(&mut rank, b, 'n');
        put(&mut rank, a, 'n');
        for c in ['r', 'k', 'r'] {
            put(&mut rank, 0, c);
        }
        let black: String = rank.iter().collect();
        format!(
            "{black}/pppppppp/8/8/8/8/PPPPPPPP/{} w KQkq - 0 1",
            black.to_uppercase()
        )
    }

    fn sm_pos(fen: &str) -> shakmaty::Chess {
        use shakmaty::{CastlingMode, fen::Fen};
        fen.parse::<Fen>()
            .unwrap()
            .into_position(CastlingMode::Chess960)
            .unwrap()
    }

    fn uci960(b: &Board, mv: Move) -> String {
        match b.castle_rook(mv) {
            Some(r) => format!("{}{}", sq_to_str(mv.from()), sq_to_str(r)),
            None => mv.to_uci(),
        }
    }

    #[test]
    fn chess960_start_positions_perft() {
        assert_eq!(
            chess960_fen(518),
            "rnbqkbnr/pppppppp/8/8/8/8/PPPPPPPP/RNBQKBNR w KQkq - 0 1"
        );
        for n in 0..960 {
            let fen = chess960_fen(n);
            let mut b = Board::from_fen(&fen).unwrap();
            assert_eq!(b.to_fen(), fen, "FEN round trip");
            assert_eq!(perft(&mut b, 3), shakmaty::perft(&sm_pos(&fen), 3), "{fen}");
        }
    }

    #[test]
    fn chess960_random_games_match_reference() {
        use shakmaty::{CastlingMode, Position};
        let mut rng = 0x9E37_79B9_7F4A_7C15u64;
        let mut next = || {
            rng ^= rng << 13;
            rng ^= rng >> 7;
            rng ^= rng << 17;
            rng
        };
        for game in 0..600 {
            let fen = chess960_fen((next() % 960) as usize);
            let mut b = Board::from_fen(&fen).unwrap();
            let start_hash = b.hash;
            let mut sm = sm_pos(&fen);
            for _ply in 0..120 {
                let mut l = MoveList::new();
                MoveGen::generate_all(&b, &mut l);
                let mut ours: Vec<String> = l.iter().map(|&m| uci960(&b, m)).collect();
                let mut theirs: Vec<String> = sm
                    .legal_moves()
                    .iter()
                    .map(|m| m.to_uci(CastlingMode::Chess960).to_string())
                    .collect();
                ours.sort();
                theirs.sort();
                assert_eq!(ours, theirs, "game {game} at {}", b.to_fen());
                if l.is_empty() {
                    break;
                }

                let rt = Board::from_fen(&b.to_fen()).unwrap();
                assert_eq!(rt.hash, b.hash, "hash drift at {}", b.to_fen());

                let castles: Vec<Move> = l
                    .iter()
                    .copied()
                    .filter(|m| b.castle_rook(*m).is_some())
                    .collect();
                let mv = if !castles.is_empty() && next() % 2 == 0 {
                    castles[(next() % castles.len() as u64) as usize]
                } else {
                    l[(next() % l.len() as u64) as usize]
                };
                let s = uci960(&b, mv);
                assert_eq!(
                    b.find_uci_move(&s),
                    Some(mv),
                    "UCI round trip of {s} at {}",
                    b.to_fen()
                );
                let smv = sm
                    .legal_moves()
                    .into_iter()
                    .find(|m| m.to_uci(CastlingMode::Chess960).to_string() == s)
                    .unwrap();
                sm.play_unchecked(smv);
                b.make_move(mv);
            }
            while !b.history.is_empty() {
                b.unmake_move();
            }
            assert_eq!(
                (b.to_fen(), b.hash),
                (fen, start_hash),
                "unmake did not restore game {game}"
            );
        }
    }

    #[test]
    fn chess960_castling_edge_cases() {
        let b = Board::from_fen("1r2k3/8/8/8/8/8/8/qRK4R w KQ - 0 1").unwrap();
        let mut l = MoveList::new();
        MoveGen::generate_all(&b, &mut l);
        assert!(l.iter().all(|m| m.flags() != MoveFlag::CastleQueen));
        assert!(l.iter().any(|m| m.flags() == MoveFlag::CastleKing));

        let mut b = Board::from_fen("4k3/8/8/8/8/8/8/4R1KR w H - 0 1").unwrap();
        let mv = b.find_uci_move("g1h1").unwrap();
        assert_eq!(mv.flags(), MoveFlag::CastleKing);
        b.make_move(mv);
        assert_eq!(b.to_fen(), "4k3/8/8/8/8/8/8/4RRK1 b - - 1 1");
        b.unmake_move();
        assert_eq!(b.to_fen(), "4k3/8/8/8/8/8/8/4R1KR w K - 0 1");

        let b = Board::from_fen("4k3/8/8/8/8/8/8/5K1R w K - 0 1").unwrap();
        assert_eq!(b.find_uci_move("f1g1").unwrap().flags(), MoveFlag::Quiet);
        assert_eq!(
            b.find_uci_move("f1h1").unwrap().flags(),
            MoveFlag::CastleKing
        );

        let b = Board::from_fen("4k3/8/8/8/8/8/8/1R2K1RR w G - 0 1").unwrap();
        assert_eq!(b.castle[0].rook, 6);
        assert!(b.to_fen().contains(" w G "));
    }
}
