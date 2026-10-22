mod bench;
mod bitboard;
mod book;
mod board;
mod datagen;
mod eval;

#[allow(dead_code)]
mod fbk;
mod genfens;
mod movegen;
mod moves;
mod nnue;
mod probe;
mod search;
mod see;
mod syzygy;
mod tt;
mod uci;
mod zobrist;

fn main() {

    nnue::load_embedded();

    let args: Vec<String> = std::env::args().collect();

    if args.get(1).map(String::as_str) == Some("bench") {
        let depth = args.get(2).and_then(|v| v.parse::<u32>().ok()).unwrap_or(bench::DEFAULT_DEPTH);
        bench::run(depth);
        return;
    }

    if let Some(fen) = parse_flag_value(&args, "--probe-fen") {
        probe::run_single(&fen);
        return;
    }
    if let Some(fen_path) = parse_flag_value(&args, "--probe") {
        let threads: usize = parse_flag_value(&args, "--probe-threads")
            .and_then(|v| v.parse::<usize>().ok())
            .unwrap_or_else(|| {
                std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4)
            });
        probe::run_batch(&fen_path, threads);
        return;
    }
    if args.len() >= 4 && args[1] == "--probe-symmetry" {
        probe::run_symmetry_check(&args[2], &args[3]);
        return;
    }

    let syzygy_path = parse_flag_value(&args, "--syzygy-path");
    let eval_path = parse_flag_value(&args, "--eval-file");
    let threads = parse_flag_value(&args, "--threads")
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.clamp(1, uci::MAX_THREADS))
        .unwrap_or(1);
    let hash_mb = parse_flag_value(&args, "--hash")
        .and_then(|v| v.parse::<usize>().ok())
        .map(|v| v.clamp(1, 1024))
        .unwrap_or(16);

    if let Some(ref path) = eval_path {
        match nnue::load(path) {
            Ok(()) => eprintln!("info string NNUE eval file loaded (overrides embedded net): {path}"),
            Err(e) => eprintln!(
                "info string Failed to load EvalFile '{path}': {e} -- keeping embedded net"
            ),
        }
    }

    if let Some(out) = parse_flag_value(&args, "--datagen") {
        let num = |flag: &str, default: u64| {
            parse_flag_value(&args, flag).and_then(|v| v.parse::<u64>().ok()).unwrap_or(default)
        };
        let seed = parse_flag_value(&args, "--seed")
            .and_then(|v| v.parse::<u64>().ok())
            .unwrap_or_else(|| book::Rng::from_time().next());
        datagen::run(datagen::Config {
            out,
            games:        num("--games", 10_000),
            threads,
            nodes:        num("--nodes", 5000).max(1),
            book:         parse_flag_value(&args, "--book")
                .filter(|b| !b.eq_ignore_ascii_case("none")),
            book_uniform: args.iter().any(|a| a == "--book-uniform"),
            random_plies: num("--random-plies", 4) as u32,
            syzygy:       syzygy_path,
            seed,
            hash_mb,
        });
        return;
    }

    if parse_flag_value(&args, "--threads").is_some() {
        eprintln!("info string Threads set to {threads} via --threads");
    }
    if parse_flag_value(&args, "--hash").is_some() {
        eprintln!("info string Hash set to {hash_mb} MB via --hash");
    }

    let commands: Vec<String> = if args.get(1).is_some_and(|a| !a.starts_with("--")) {
        args[1..].to_vec()
    } else {
        Vec::new()
    };

    uci::uci_loop(syzygy_path, threads, hash_mb, commands);
}

fn parse_flag_value(args: &[String], flag: &str) -> Option<String> {
    let eq_prefix = format!("{flag}=");
    let mut iter = args.iter().peekable();
    while let Some(arg) = iter.next() {
        if arg == flag {
            return iter.next().cloned();
        }
        if let Some(val) = arg.strip_prefix(eq_prefix.as_str()) {
            return Some(val.to_string());
        }
    }
    None
}
