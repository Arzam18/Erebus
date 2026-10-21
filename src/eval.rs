

use crate::board::Board;

pub const INF:        i32 = 1_000_000;
pub const MATE_SCORE: i32 = 900_000;
pub const DRAW_SCORE: i32 = 0;

pub fn evaluate(board: &mut Board) -> i32 {
    crate::nnue::evaluate(board)
}
