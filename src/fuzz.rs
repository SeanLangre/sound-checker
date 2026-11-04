//! Tiny deterministic PRNG and input generators for the randomized tests.
//! Set `FUZZ_SEED` / `FUZZ_ITERS` to reproduce or deepen a run.

pub struct Rng(u64);

impl Rng {
    pub fn new(salt: u64) -> Self {
        let seed = std::env::var("FUZZ_SEED").ok().and_then(|s| s.parse().ok()).unwrap_or(0x5EED);
        Rng(seed ^ salt.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    pub fn u64(&mut self) -> u64 {
        // xorshift64*
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    pub fn below(&mut self, n: usize) -> usize {
        (self.u64() % n.max(1) as u64) as usize
    }
    pub fn chance(&mut self, pct: usize) -> bool {
        self.below(100) < pct
    }
    pub fn pick<'a, T>(&mut self, xs: &'a [T]) -> &'a T {
        &xs[self.below(xs.len())]
    }
    /// Uniform in [-1, 1).
    pub fn unit(&mut self) -> f32 {
        (self.u64() >> 40) as f32 / (1u64 << 23) as f32 - 1.0
    }
    /// A float, biased toward values that break audio code.
    pub fn nasty_f32(&mut self) -> f32 {
        match self.below(14) {
            0 => f32::NAN,
            1 => f32::INFINITY,
            2 => f32::NEG_INFINITY,
            3 => f32::MAX,
            4 => f32::MIN,
            5 => f32::MIN_POSITIVE / 2.0, // subnormal
            6 => -0.0,
            7 => 1.0,
            8 => -1.0,
            9 => 1e20,
            10 => 0.0,
            _ => self.unit(),
        }
    }
    pub fn string(&mut self, max_len: usize) -> String {
        const BITS: &[&str] = &[
            "\"", "\\", "\n", "\r", "\t", "\0", "%", "%f", "$", "`", " ", "'", "{", "}", "[", "]",
            "=", "#", ":", "é", "日本", "😀", "\u{202e}", "\u{2028}", "bluez_", "alsa_input.",
            "Device ", "Icon: audio-", "Connected: yes", "bluez_card.", "00:11:22:33:44:55",
            "74_B7_E6_D6_60_0D", "bluetooth", "a", "Z", "0",
        ];
        let n = self.below(max_len + 1);
        let mut s = String::new();
        for _ in 0..n {
            if self.chance(70) {
                s.push_str(self.pick(BITS));
            } else {
                s.push(char::from_u32(self.u64() as u32 % 0x11_0000).unwrap_or('?'));
            }
        }
        s
    }
    pub fn bytes(&mut self, max_len: usize) -> Vec<u8> {
        (0..self.below(max_len + 1)).map(|_| self.u64() as u8).collect()
    }
}

pub fn iters(default: usize) -> usize {
    std::env::var("FUZZ_ITERS").ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}
