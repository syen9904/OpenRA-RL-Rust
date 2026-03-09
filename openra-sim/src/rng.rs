//! Mersenne Twister PRNG — bit-for-bit copy of OpenRA's MersenneTwister.cs.
//! Any deviation here causes cascading desync in all downstream systems.
//!
//! Reference: OpenRA.Game/Support/MersenneTwister.cs

const N: usize = 624;

#[derive(Clone)]
pub struct MersenneTwister {
    mt: [u32; N],
    index: usize,
    pub last: i32,
    pub total_count: i32,
}

impl MersenneTwister {
    pub fn new(seed: i32) -> Self {
        let mut mt = [0u32; N];
        mt[0] = seed as u32;
        for i in 1..N {
            mt[i] = 1812433253u32
                .wrapping_mul(mt[i - 1] ^ (mt[i - 1] >> 30))
                .wrapping_add(i as u32);
        }
        MersenneTwister {
            mt,
            index: 0,
            last: 0,
            total_count: 0,
        }
    }

    /// Produces a random unsigned 32-bit integer.
    pub fn next_uint(&mut self) -> u32 {
        if self.index == 0 {
            self.generate();
        }

        let mut y = self.mt[self.index];
        y ^= y >> 11;
        y ^= (y << 7) & 2636928640;
        y ^= (y << 15) & 4022730752;
        y ^= y >> 18;

        self.index = (self.index + 1) % N;
        self.total_count += 1;
        self.last = (y % (i32::MAX as u32)) as i32;
        y
    }

    /// Produces a random unsigned 64-bit integer.
    pub fn next_ulong(&mut self) -> u64 {
        (self.next_uint() as u64) << 32 | self.next_uint() as u64
    }

    /// Produces signed integers between -0x7fffffff and 0x7fffffff inclusive.
    /// 0 is twice as likely as any other number.
    pub fn next(&mut self) -> i32 {
        self.next_uint();
        self.last
    }

    /// Produces signed integers in [low, high).
    pub fn next_range(&mut self, low: i32, high: i32) -> i32 {
        assert!(high >= low, "Maximum value is less than the minimum value.");
        let diff = high - low;
        if diff <= 1 {
            return low;
        }
        low + self.next() % diff
    }

    fn generate(&mut self) {
        for i in 0..N {
            let y = (self.mt[i] & 0x80000000) | (self.mt[(i + 1) % N] & 0x7fffffff);
            self.mt[i] = self.mt[(i + 397) % N] ^ (y >> 1);
            if (y & 1) == 1 {
                self.mt[i] ^= 2567483615;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seed_zero_first_values() {
        // Verify the MT produces consistent values for seed=0
        let mut rng = MersenneTwister::new(0);
        let v0 = rng.next_uint();
        let v1 = rng.next_uint();
        let v2 = rng.next_uint();

        // These are deterministic — same seed always produces same sequence
        assert_eq!(v0, 2357136044);
        assert_eq!(v1, 2546248239);
        assert_eq!(v2, 3071714933);
        assert_eq!(rng.total_count, 3);
    }

    #[test]
    fn seed_42_first_values() {
        let mut rng = MersenneTwister::new(42);
        let v0 = rng.next_uint();
        let v1 = rng.next_uint();
        let v2 = rng.next_uint();

        assert_eq!(v0, 1608637542);
        assert_eq!(v1, 3421126067);
        assert_eq!(v2, 4083286876);
    }

    #[test]
    fn next_returns_last() {
        let mut rng = MersenneTwister::new(0);
        let val = rng.next();
        assert_eq!(val, rng.last);
    }

    #[test]
    fn last_is_y_mod_int_max() {
        // C#: Last = (int)(y % int.MaxValue)
        let mut rng = MersenneTwister::new(0);
        let y = rng.next_uint();
        // last should be y % 0x7fffffff
        let expected = (2357136044u32 % (i32::MAX as u32)) as i32;
        assert_eq!(rng.last, expected);
        let _ = y;
    }

    #[test]
    fn next_range_bounds() {
        let mut rng = MersenneTwister::new(123);
        for _ in 0..100 {
            let val = rng.next_range(5, 10);
            assert!(val >= 5 && val < 10, "val={val} out of [5,10)");
        }
    }

    #[test]
    fn next_range_equal_returns_low() {
        let mut rng = MersenneTwister::new(0);
        assert_eq!(rng.next_range(7, 7), 7);
        assert_eq!(rng.next_range(7, 8), 7);
    }

    #[test]
    fn generate_wraps_index() {
        // After 624 calls, index wraps and generate() is called again
        let mut rng = MersenneTwister::new(0);
        for _ in 0..624 {
            rng.next_uint();
        }
        assert_eq!(rng.index, 0);
        // 625th call should still work (triggers generate again)
        let _ = rng.next_uint();
        assert_eq!(rng.total_count, 625);
    }

    #[test]
    fn next_ulong_uses_two_calls() {
        let mut rng1 = MersenneTwister::new(99);
        let ulong_val = rng1.next_ulong();
        assert_eq!(rng1.total_count, 2);

        let mut rng2 = MersenneTwister::new(99);
        let hi = rng2.next_uint() as u64;
        let lo = rng2.next_uint() as u64;
        assert_eq!(ulong_val, (hi << 32) | lo);
    }
}
