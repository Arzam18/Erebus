

use std::fs;
use std::io::{BufRead, BufReader};
use std::thread;
use std::time::Instant;

use crate::board::{Color, Piece};
use crate::nnue::{self, DualAccumulator};

type SqPieces = [Option<(Piece, Color)>; 64];

fn parse_fen_for_eval(fen: &str) -> Option<(SqPieces, Color)> {
    let mut sq_piece: SqPieces = [None; 64];
    let mut kings = [false; 2];

    let mut parts = fen.split_whitespace();
    let placement = parts.next()?;
    let stm_str = parts.next().unwrap_or("w");

    let mut rank: i32 = 7;
    let mut file: i32 = 0;
    for c in placement.chars() {
        match c {
            '/' => { rank -= 1; file = 0; }
            '1'..='8' => file += (c as i32) - ('0' as i32),
            _ => {
                if let Some((piece, color)) = Piece::from_char(c) {
                    if !(0..8).contains(&rank) || !(0..8).contains(&file) {
                        return None;
                    }
                    let sq = (rank * 8 + file) as u8;
                    sq_piece[sq as usize] = Some((piece, color));
                    if piece == Piece::King {
                        kings[color as usize] = true;
                    }
                    file += 1;
                }
            }
        }
    }
    if kings != [true, true] {
        return None;
    }
    let stm = if stm_str == "b" { Color::Black } else { Color::White };
    Some((sq_piece, stm))
}

fn eval_fen(net: &nnue::Net, fen: &str) -> Option<i32> {
    let (sq_piece, stm) = parse_fen_for_eval(fen)?;
    let mut acc = DualAccumulator::zeroed();
    nnue::refresh_all(net, &sq_piece, &mut acc);
    let pieces = sq_piece.iter().filter(|s| s.is_some()).count() as u32;
    Some(nnue::evaluate_with_net(net, &acc, stm, pieces))
}

pub fn run_single(fen: &str) {
    let guard = nnue::read_lock();
    let net = guard.as_ref().expect("net not loaded");
    match eval_fen(net, fen) {
        Some(cp) => println!("{cp}"),
        None => {
            eprintln!("Could not parse FEN: {fen}");
            std::process::exit(1);
        }
    }
}

pub fn run_symmetry_check(fen: &str, mirrored_fen: &str) {
    let guard = nnue::read_lock();
    let net = guard.as_ref().expect("net not loaded");
    let a = eval_fen(net, fen).unwrap_or_else(|| {
        eprintln!("Could not parse FEN: {fen}");
        std::process::exit(1);
    });
    let b = eval_fen(net, mirrored_fen).unwrap_or_else(|| {
        eprintln!("Could not parse mirrored FEN: {mirrored_fen}");
        std::process::exit(1);
    });
    println!("eval(fen)                      = {a}");
    println!("eval(mirrored_fen)             = {b}");
    println!("eval(fen) - eval(mirrored_fen) = {} (want 0)", a - b);
}

pub fn run_batch(fen_path: &str, threads: usize) {
    let file = fs::File::open(fen_path).unwrap_or_else(|e| {
        eprintln!("Could not open '{fen_path}': {e}");
        std::process::exit(1);
    });
    let lines: Vec<String> = BufReader::new(file)
        .lines()
        .map_while(Result::ok)
        .filter(|l| !l.trim().is_empty())
        .collect();

    let total_lines = lines.len();
    if total_lines == 0 {
        eprintln!("No non-empty lines found in '{fen_path}'");
        return;
    }

    let start = Instant::now();
    let threads = threads.max(1);
    let chunk_size = total_lines.div_ceil(threads).max(1);
    let mut handles = Vec::new();

    for (chunk_idx, chunk) in lines.chunks(chunk_size).map(|c| c.to_vec()).enumerate() {
        handles.push(thread::spawn(move || {

            let guard = nnue::read_lock();
            let net = guard.as_ref().expect("net not loaded");
            let mut ok = 0u64;
            let mut bad = 0u64;
            for (i, line) in chunk.iter().enumerate() {
                match eval_fen(net, line) {
                    Some(_) => ok += 1,
                    None => {
                        bad += 1;
                        if bad <= 5 {
                            eprintln!(
                                "info string [chunk {chunk_idx} line {i}] skipped malformed FEN: {line}"
                            );
                        }
                    }
                }
            }
            (ok, bad)
        }));
    }

    let (total_ok, total_bad) = handles
        .into_iter()
        .map(|h| h.join().unwrap())
        .fold((0u64, 0u64), |(a, b), (x, y)| (a + x, b + y));

    let elapsed = start.elapsed();
    let pos_per_sec = total_ok as f64 / elapsed.as_secs_f64().max(1e-9);
    println!(
        "Evaluated {total_ok}/{total_lines} positions in {:.2}s ({pos_per_sec:.0} pos/s, {threads} threads, {total_bad} skipped)",
        elapsed.as_secs_f64()
    );
}
