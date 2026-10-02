//! Bit-level reading of parameter sets and slice headers.
use crate::error::Error;

pub fn bad(message: &str) -> Error {
    Error::Corrupted(format!("HEVC: {message}"))
}

/// Removes the emulation prevention bytes (`00 00 03` becomes `00 00`) from a
/// NAL unit payload. Returns the data and, for each removed byte, the position
/// in the returned data where it was dropped (entry points count them).
pub fn unescape(escaped: &[u8]) -> (Vec<u8>, Vec<usize>) {
    let mut data = Vec::with_capacity(escaped.len());
    let mut removed = Vec::new();
    let mut zeros = 0;
    for &byte in escaped {
        if zeros >= 2 && byte == 3 {
            removed.push(data.len());
            zeros = 0;
            continue;
        }
        zeros = if byte == 0 { zeros + 1 } else { 0 };
        data.push(byte);
    }
    (data, removed)
}

pub struct Bits<'a> {
    data: &'a [u8],
    position: usize,
}
impl<'a> Bits<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data, position: 0 }
    }
    pub fn bits_left(&self) -> usize {
        self.data.len() * 8 - self.position.min(self.data.len() * 8)
    }
    pub fn flag(&mut self) -> Result<bool, Error> {
        Ok(self.u(1)? == 1)
    }
    /// `count` bits (at most 32), most significant first.
    pub fn u(&mut self, count: u32) -> Result<u32, Error> {
        if count > 32 || self.bits_left() < count as usize {
            return Err(bad("a header ends early"));
        }
        let mut value = 0u64;
        for _ in 0..count {
            let byte = self.data[self.position / 8];
            value = value << 1 | u64::from(byte >> (7 - self.position % 8) & 1);
            self.position += 1;
        }
        Ok(value as u32)
    }
    pub fn skip(&mut self, count: usize) -> Result<(), Error> {
        if self.bits_left() < count {
            return Err(bad("a header ends early"));
        }
        self.position += count;
        Ok(())
    }
    /// Unsigned Exp-Golomb code.
    pub fn ue(&mut self) -> Result<u32, Error> {
        let mut zeros = 0;
        while !self.flag()? {
            zeros += 1;
            if zeros > 32 {
                return Err(bad("an Exp-Golomb code is too long"));
            }
        }
        if zeros == 0 {
            return Ok(0);
        }
        let rest = u64::from(self.u(zeros.min(32))?);
        let value = (1u64 << zeros) - 1 + rest;
        u32::try_from(value).map_err(|_| bad("an Exp-Golomb value is too large"))
    }
    /// Signed Exp-Golomb code.
    pub fn se(&mut self) -> Result<i32, Error> {
        let k = i64::from(self.ue()?);
        Ok(if k % 2 == 1 { (k + 1) / 2 } else { -(k / 2) } as i32)
    }
    pub fn byte_align(&mut self) {
        self.position = self.position.div_ceil(8) * 8;
    }
    /// Index of the next whole byte.
    pub fn byte_position(&self) -> usize {
        self.position.div_ceil(8)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn exp_golomb_codes() {
        // 1 | 010 | 011 | 00100 | 00101 -> 0, 1, 2, 3, 4
        let mut bits = Bits::new(&[0b1010_0110, 0b0100_0010, 0b1000_0000]);
        for expected in 0..5 {
            assert_eq!(bits.ue().unwrap(), expected);
        }
        // se: 1 -> 0, 010 -> 1, 011 -> -1, 00100 -> 2
        let mut bits = Bits::new(&[0b1010_0110, 0b0100_0000]);
        assert_eq!(
            [
                bits.se().unwrap(),
                bits.se().unwrap(),
                bits.se().unwrap(),
                bits.se().unwrap()
            ],
            [0, 1, -1, 2]
        );
    }
    #[test]
    fn emulation_prevention_is_removed_and_counted() {
        let (data, removed) = unescape(&[0, 0, 3, 1, 0, 0, 3, 0, 0, 3]);
        assert_eq!(data, [0, 0, 1, 0, 0, 0, 0]);
        assert_eq!(removed, [2, 5, 7]);
        // A 3 after a single zero is data.
        assert_eq!(unescape(&[0, 3, 5]).0, [0, 3, 5]);
    }
    #[test]
    fn reading_past_the_end_is_an_error() {
        let mut bits = Bits::new(&[0xff]);
        assert_eq!(bits.u(8).unwrap(), 255);
        assert!(bits.u(1).is_err());
        assert!(Bits::new(&[0, 0, 0, 0, 0]).ue().is_err());
    }
}
