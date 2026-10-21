

use std::num::NonZeroU32;
use std::path::Path;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use shakmaty::{
    Bitboard as SmBB, Board as SmBoard, CastlingMode, Chess, Color as SmColor,
    File as SmFile, FromSetup, Piece as SmPiece, PositionError,
    Rank as SmRank, Role, Setup, Square as SmSquare,
};
use shakmaty_syzygy::{MaybeRounded, Tablebase, Wdl};

use crate::board::{Board, Color, Piece};

#[derive(Copy, Clone, PartialEq, Eq, Debug)]
pub enum WdlResult {
    Win,
    Draw,
    Loss,

    CursedWin,

    BlessedLoss,
}

impl WdlResult {

    pub fn without_rule50(self) -> WdlResult {
        match self {
            WdlResult::CursedWin   => WdlResult::Win,
            WdlResult::BlessedLoss => WdlResult::Loss,
            w => w,
        }
    }
}

pub static PROBE_DEPTH: AtomicU32 = AtomicU32::new(1);

pub static PROBE_LIMIT: AtomicU32 = AtomicU32::new(7);

pub static RULE50: AtomicBool = AtomicBool::new(true);

pub fn probe_depth() -> u32 { PROBE_DEPTH.load(Ordering::Relaxed) }
pub fn rule50() -> bool { RULE50.load(Ordering::Relaxed) }

pub fn count_files(dir: &str) -> (usize, usize) {
    let (mut wdl, mut dtz) = (0, 0);
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten() {
            match e.path().extension().and_then(|x| x.to_str()) {
                Some("rtbw") => wdl += 1,
                Some("rtbz") => dtz += 1,
                _ => {}
            }
        }
    }
    (wdl, dtz)
}

pub const TB_WIN_SCORE:  i32 = 800_000;
pub const TB_LOSS_SCORE: i32 = -800_000;

pub fn tb_wdl_score(wdl: WdlResult, ply: usize) -> i32 {
    match wdl {
        WdlResult::Win         => TB_WIN_SCORE - ply as i32,
        WdlResult::Loss        => TB_LOSS_SCORE + ply as i32,
        WdlResult::CursedWin   => 1,
        WdlResult::BlessedLoss => -1,
        WdlResult::Draw        => 0,
    }
}

pub struct SyzygyTb {
    tables: Tablebase<Chess>,
}

impl SyzygyTb {
    pub fn new() -> Self {
        SyzygyTb { tables: Tablebase::new() }
    }

    pub fn add_directory(&mut self, path: &str) -> usize {
        match self.tables.add_directory(Path::new(path)) {
            Ok(n)  => n,
            Err(e) => {
                eprintln!("info string Syzygy error loading {}: {}", path, e);
                0
            }
        }
    }

    pub fn is_loaded(&self) -> bool {
        self.tables.max_pieces() > 0
    }

    pub fn max_pieces(&self) -> usize {
        self.tables.max_pieces()
    }

    pub fn card(&self) -> usize {
        self.max_pieces().min(PROBE_LIMIT.load(Ordering::Relaxed) as usize)
    }

    pub fn probe_wdl(&self, board: &Board) -> Option<WdlResult> {
        if !self.should_probe(board) { return None; }
        let pos = board_to_chess(board)?;
        match self.tables.probe_wdl_after_zeroing(&pos) {
            Ok(wdl) => Some(convert_wdl(wdl)),
            Err(e)  => {
                eprintln!("info string Syzygy WDL probe error: {}", e);
                None
            }
        }
    }

    pub fn probe_dtz_root(&self, board: &Board) -> Option<i32> {
        if !self.is_loaded()
            || board.occupied().count_ones() as usize > self.max_pieces()
            || board.castling != 0
        {
            return None;
        }
        let pos = board_to_chess(board)?;
        match self.tables.probe_dtz(&pos) {
            Ok(MaybeRounded::Precise(dtz)) | Ok(MaybeRounded::Rounded(dtz)) => Some(dtz.0),
            Err(_) => None,
        }
    }

    #[inline]
    fn should_probe(&self, board: &Board) -> bool {
        if !self.is_loaded() { return false; }
        let piece_count = board.occupied().count_ones() as usize;
        piece_count <= self.max_pieces() && board.halfmove == 0 && board.castling == 0
    }
}

#[allow(clippy::result_large_err)]
pub(crate) fn board_to_chess(board: &Board) -> Option<Chess> {
    let sm_board      = build_sm_board(board);
    let turn          = if board.side == Color::White { SmColor::White } else { SmColor::Black };
    let ep_square     = board.ep_sq.map(sq_to_sm);
    let castling_rights = build_castling_rights(board);
    let halfmoves     = board.halfmove as u32;
    let fullmoves     = NonZeroU32::new(board.fullmove as u32)
                            .unwrap_or(NonZeroU32::MIN);

    let setup = Setup {
        board:            sm_board,
        turn,
        castling_rights,
        ep_square,
        halfmoves,
        fullmoves,

        promoted:         SmBB::EMPTY,
        pockets:          None,
        remaining_checks: None,
    };

    Chess::from_setup(setup, CastlingMode::Standard)
        .or_else(|e: PositionError<Chess>| {
            e.ignore_invalid_ep_square()
        })
        .or_else(|e: PositionError<Chess>| {
            e.ignore_invalid_castling_rights()
        })
        .ok()
}

fn build_sm_board(board: &Board) -> SmBoard {
    let mut sm = SmBoard::empty();
    for sq in 0u8..64 {
        if let Some((piece, color)) = board.sq_piece[sq as usize] {
            let role = match piece {
                Piece::Pawn   => Role::Pawn,
                Piece::Knight => Role::Knight,
                Piece::Bishop => Role::Bishop,
                Piece::Rook   => Role::Rook,
                Piece::Queen  => Role::Queen,
                Piece::King   => Role::King,
            };
            let color = if color == Color::White { SmColor::White } else { SmColor::Black };
            sm.set_piece_at(sq_to_sm(sq), SmPiece { role, color });
        }
    }
    sm
}

fn build_castling_rights(board: &Board) -> SmBB {
    let mut bb = SmBB::EMPTY;
    for idx in 0..4 {
        if board.castling & (1 << idx) != 0 { bb |= SmBB::from_square(sq_to_sm(board.castle[idx].rook)); }
    }
    bb
}

#[inline]
fn sq_to_sm(sq: u8) -> SmSquare {
    let file = SmFile::new((sq & 7) as u32);
    let rank = SmRank::new((sq >> 3) as u32);
    SmSquare::from_coords(file, rank)
}

#[inline]
fn convert_wdl(wdl: Wdl) -> WdlResult {
    match wdl {
        Wdl::Win         => WdlResult::Win,
        Wdl::Draw        => WdlResult::Draw,
        Wdl::Loss        => WdlResult::Loss,
        Wdl::CursedWin   => WdlResult::CursedWin,
        Wdl::BlessedLoss => WdlResult::BlessedLoss,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursed_and_blessed_score_as_draws() {
        assert_eq!(tb_wdl_score(WdlResult::CursedWin, 5), 1);
        assert_eq!(tb_wdl_score(WdlResult::BlessedLoss, 5), -1);
        assert_eq!(tb_wdl_score(WdlResult::Draw, 5), 0);

        let fast = tb_wdl_score(WdlResult::Win, 3);
        let slow = tb_wdl_score(WdlResult::Win, 9);
        assert!(fast > slow && slow > 700_000);
        assert!(tb_wdl_score(WdlResult::Loss, 3) < tb_wdl_score(WdlResult::Loss, 9));
    }
}
