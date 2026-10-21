

pub struct ZobristKeys {
    pub piece:   [[[u64; 64]; 6]; 2],
    pub side:    u64,
    pub castle:  [u64; 4],
    pub ep_file: [u64; 8],
}

impl ZobristKeys {
    pub const fn new() -> Self {

        let mut state: u64 = 0xDEADBEEF_CAFEBABE;
        macro_rules! next {
            () => {{
                state = state.wrapping_mul(6364136223846793005)
                             .wrapping_add(1442695040888963407);
                state
            }};
        }

        let mut piece = [[[0u64; 64]; 6]; 2];
        let mut c = 0;
        while c < 2 {
            let mut p = 0;
            while p < 6 {
                let mut s = 0;
                while s < 64 {
                    piece[c][p][s] = next!();
                    s += 1;
                }
                p += 1;
            }
            c += 1;
        }

        let side = next!();

        let mut castle = [0u64; 4];
        let mut i = 0;
        while i < 4 { castle[i] = next!(); i += 1; }

        let mut ep_file = [0u64; 8];
        let mut i = 0;
        while i < 8 { ep_file[i] = next!(); i += 1; }

        ZobristKeys { piece, side, castle, ep_file }
    }
}

pub static ZOBRIST: ZobristKeys = ZobristKeys::new();
