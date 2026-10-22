//! `genfens N seed S book <path|None>`: opening positions for OpenBench.
//!
//! OpenBench runs `erebus "genfens N seed S book <book>" "quit"` and collects
//! every `info string genfens <fen>` line as an opening for its games. Each
//! position is a start (the start position, or a random line of the EPD book)
//! followed by a few uniformly random legal moves, kept only if a short
//! verification search finds it roughly balanced, so the openings are varied
//! but not already decided.
//!
//! The output depends only on N, the seed, the book and the net: every
//! verification search gets a cleared hash table and fresh move-ordering
//! tables, as in `bench`.

use std::collections::HashSet;
use std::io::Write;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64};

use crate::board::Board;
use crate::book::Rng;
use crate::movegen::MoveGen;
use crate::moves::MoveList;
use crate::search::{SearchLimits, SearchTables, Searcher};
use crate::tt::TranspositionTable;

/// Random plies from the start position: 8 or 9, so both sides get to move.
const STARTPOS_PLIES: u32 = 8;
/// Random plies after a book line: 2 or 3, keeping the book's character.
const BOOK_PLIES: u32 = 2;
/// Soft node budget of the verification search.
const VERIFY_NODES: u64 = 5000;
/// Reject positions whose verification score is beyond this (centipawns).
const MAX_SCORE: i32 = 1000;
const HASH_MB: usize = 8;
/// Give up after this many rejected attempts in a row (a broken book).
const MAX_REJECTS: u32 = 100_000;

/// Handle `genfens N seed S book <path|None> [extra...]` (tokens include
/// `genfens`). Extra trailing arguments are ignored.
pub fn run(tokens: &[&str]) {
    let value = |key: &str| {
        tokens.iter().position(|t| t.eq_ignore_ascii_case(key)).and_then(|i| tokens.get(i + 1)).copied()
    };
    let Some(count) = tokens.get(1).and_then(|v| v.parse::<usize>().ok()) else {
        eprintln!("info string genfens: usage: genfens <N> seed <S> book <path|None>");
        return;
    };
    let seed = value("seed").and_then(|v| v.parse::<u64>().ok()).unwrap_or(0);
    let book = match value("book").filter(|b| !b.eq_ignore_ascii_case("none")) {
        Some(path) => match EpdBook::load(path) {
            Ok(book) if !book.is_empty() => book,
            Ok(_) => {
                eprintln!("info string genfens: book '{path}' has no positions");
                return;
            }
            Err(e) => {
                eprintln!("info string genfens: cannot read book '{path}': {e}");
                return;
            }
        },
        None => EpdBook::empty(),
    };

    let mut rng = Rng::new(seed ^ 0x9E37_79B9_7F4A_7C15);
    let tt = Arc::new(TranspositionTable::new(HASH_MB));
    let mut seen = HashSet::new();
    let mut rejects = 0;
    let stdout = std::io::stdout();
    let mut out = stdout.lock();

    while seen.len() < count {
        match candidate(&book, &mut rng, &tt) {
            Some(fen) if seen.insert(fen.clone()) => {
                rejects = 0;
                let _ = writeln!(out, "info string genfens {fen}");
            }
            _ => {
                rejects += 1;
                if rejects >= MAX_REJECTS {
                    eprintln!("info string genfens: {MAX_REJECTS} positions rejected in a row, stopping");
                    break;
                }
            }
        }
    }
    let _ = out.flush();
}

/// One random opening, or `None` if it ends the game, is unbalanced, or the
/// book line does not parse.
fn candidate(book: &EpdBook, rng: &mut Rng, tt: &Arc<TranspositionTable>) -> Option<String> {
    let (mut board, base) = if book.is_empty() {
        (Board::start_pos(), STARTPOS_PLIES)
    } else {
        (Board::from_fen(book.line((rng.next() % book.lines.len() as u64) as usize))?, BOOK_PLIES)
    };
    let plies = base + (rng.next() & 1) as u32;
    let mut list = MoveList::new();
    for _ in 0..plies {
        list.truncate(0);
        MoveGen::generate_all(&board, &mut list);
        if list.is_empty() {
            return None;
        }
        board.make_move(list[(rng.next() % list.len() as u64) as usize]);
    }
    list.truncate(0);
    MoveGen::generate_all(&board, &mut list);
    if list.is_empty() {
        return None;
    }

    tt.clear();
    let limits = SearchLimits {
        soft_nodes: Some(VERIFY_NODES),
        nodes:      Some(VERIFY_NODES * 4),
        quiet:      true,
        ..SearchLimits::default()
    };
    let mut searcher = Searcher::new(
        limits,
        Arc::clone(tt),
        Arc::new(None),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicU64::new(0)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicBool::new(false)),
        SearchTables::default(),
        0,
    );
    searcher.search(&mut board);
    if searcher.score().abs() > MAX_SCORE {
        return None;
    }
    Some(board.to_fen())
}

/// An EPD/FEN book: the whole file in one buffer plus the byte range of each
/// non-empty, non-comment line. OpenBench books run to hundreds of MB, and a
/// String per line costs seconds of allocation -- enough, with one engine per
/// thread loading at once, to trip OpenBench's 15 s genfens stall timeout.
/// Board::from_fen reads the first four fields and tolerates EPD opcodes in
/// place of the move counters.
struct EpdBook {
    text:  String,
    lines: Vec<(usize, usize)>,
}

impl EpdBook {
    fn load(path: &str) -> std::io::Result<EpdBook> {
        let text = std::fs::read_to_string(path)?;
        let mut lines = Vec::new();
        let mut start = 0;
        for line in text.split_inclusive('\n') {
            let trimmed = line.trim();
            if !trimmed.is_empty() && !trimmed.starts_with('#') {
                let off = start + (trimmed.as_ptr() as usize - line.as_ptr() as usize);
                lines.push((off, off + trimmed.len()));
            }
            start += line.len();
        }
        Ok(EpdBook { text, lines })
    }

    fn empty() -> EpdBook {
        EpdBook { text: String::new(), lines: Vec::new() }
    }

    fn is_empty(&self) -> bool {
        self.lines.is_empty()
    }

    fn line(&self, i: usize) -> &str {
        let (a, b) = self.lines[i];
        &self.text[a..b]
    }
}
