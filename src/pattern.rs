//! Deterministic, reproducible test data. Every 8-byte word is derived from
//! its absolute byte offset, so any block can be regenerated for verification
//! without storing anything.

#[inline]
fn splitmix64(x: u64) -> u64 {
    let mut z = x.wrapping_add(0x9E37_79B9_7F4A_7C15);
    z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    z ^ (z >> 31)
}

/// Fill `buf` with the pattern for absolute byte `offset` in the test stream.
/// Both `offset` and `buf.len()` must be multiples of 8.
pub fn fill(buf: &mut [u8], offset: u64) {
    debug_assert!(offset % 8 == 0 && buf.len() % 8 == 0);
    for (i, chunk) in buf.chunks_exact_mut(8).enumerate() {
        let word = splitmix64(offset + (i as u64) * 8);
        chunk.copy_from_slice(&word.to_le_bytes());
    }
}

/// Byte index (within `buf`) of the first deviation from the expected pattern.
pub fn first_mismatch(buf: &[u8], offset: u64) -> Option<usize> {
    for (i, chunk) in buf.chunks_exact(8).enumerate() {
        let expected = splitmix64(offset + (i as u64) * 8).to_le_bytes();
        if chunk != expected {
            let j = chunk
                .iter()
                .zip(expected.iter())
                .position(|(a, b)| a != b)
                .unwrap_or(0);
            return Some(i * 8 + j);
        }
    }
    None
}

/// Cheap non-cryptographic RNG for benchmark offsets (xorshift64*).
pub struct Rng(u64);

impl Rng {
    pub fn from_time() -> Self {
        let seed = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x5EED)
            | 1;
        Rng(seed)
    }

    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fill_is_deterministic_and_offset_addressable() {
        let mut whole = vec![0u8; 64];
        fill(&mut whole, 0);
        let mut tail = vec![0u8; 32];
        fill(&mut tail, 32);
        assert_eq!(&whole[32..], &tail[..]);
    }

    #[test]
    fn mismatch_detection_pinpoints_byte() {
        let mut buf = vec![0u8; 64];
        fill(&mut buf, 4096);
        assert_eq!(first_mismatch(&buf, 4096), None);
        buf[17] ^= 0xFF;
        assert_eq!(first_mismatch(&buf, 4096), Some(17));
    }
}
