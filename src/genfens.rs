

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

const STARTPOS_PLIES: u32 = 8;

const BOOK_PLIES: u32 = 2;

const VERIFY_NODES: u64 = 5000;

const MAX_SCORE: i32 = 1000;
const HASH_MB: usize = 8;

const MAX_REJECTS: u32 = 100_000;

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
        Some(path) => match load_epd(path) {
            Ok(lines) if !lines.is_empty() => lines,
            Ok(_) => {
                eprintln!("info string genfens: book '{path}' has no positions");
                return;
            }
            Err(e) => {
                eprintln!("info string genfens: cannot read book '{path}': {e}");
                return;
            }
        },
        None => Vec::new(),
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

fn candidate(book: &[String], rng: &mut Rng, tt: &Arc<TranspositionTable>) -> Option<String> {
    let (mut board, base) = if book.is_empty() {
        (Board::start_pos(), STARTPOS_PLIES)
    } else {
        (Board::from_fen(&book[(rng.next() % book.len() as u64) as usize])?, BOOK_PLIES)
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

fn load_epd(path: &str) -> std::io::Result<Vec<String>> {
    Ok(std::fs::read_to_string(path)?
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect())
}
