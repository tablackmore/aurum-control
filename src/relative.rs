//! Relative (endless-encoder) Control Change decoding.
//!
//! Endless rotary encoders send a *delta* rather than an absolute position. There
//! are three encodings in common use across DJ software (Mixxx, Traktor, Serato).
//! All map a 7-bit value `0..=127` to a signed tick delta.

use serde::{Deserialize, Serialize};

/// How an endless encoder encodes its direction + magnitude into a CC value.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RelEncoding {
    /// Signed 7-bit two's-complement: `1..=63` = +, `127..=65` = − (Traktor "7Fh/01h").
    TwosComplement,
    /// Offset by 64: `delta = v - 64` (Traktor "3Fh/41h", Mixxx `SelectKnob`).
    BinaryOffset,
    /// Bit 6 is the sign (set = positive), bits 0–5 the magnitude
    /// (Serato/Bitwig "relative signed bit").
    SignedBit,
}

/// Decode one relative CC value into a signed tick delta.
pub fn decode_relative(enc: RelEncoding, v: u8) -> i32 {
    let v = (v & 0x7F) as i32;
    match enc {
        RelEncoding::TwosComplement => {
            if v < 64 {
                v
            } else {
                v - 128
            }
        }
        RelEncoding::BinaryOffset => v - 64,
        RelEncoding::SignedBit => {
            let mag = v & 0x3F;
            if v & 0x40 != 0 {
                mag
            } else {
                -mag
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn twos_complement_boundaries() {
        use RelEncoding::TwosComplement as TC;
        assert_eq!(decode_relative(TC, 0), 0);
        assert_eq!(decode_relative(TC, 1), 1);
        assert_eq!(decode_relative(TC, 63), 63);
        assert_eq!(decode_relative(TC, 127), -1);
        assert_eq!(decode_relative(TC, 65), -63);
        assert_eq!(decode_relative(TC, 64), -64);
    }

    #[test]
    fn binary_offset_boundaries() {
        use RelEncoding::BinaryOffset as BO;
        assert_eq!(decode_relative(BO, 64), 0);
        assert_eq!(decode_relative(BO, 65), 1);
        assert_eq!(decode_relative(BO, 63), -1);
        assert_eq!(decode_relative(BO, 0), -64);
        assert_eq!(decode_relative(BO, 127), 63);
    }

    #[test]
    fn signed_bit_boundaries() {
        use RelEncoding::SignedBit as SB;
        assert_eq!(decode_relative(SB, 0x41), 1);
        assert_eq!(decode_relative(SB, 0x7F), 63);
        assert_eq!(decode_relative(SB, 0x01), -1);
        assert_eq!(decode_relative(SB, 0x3F), -63);
        assert_eq!(decode_relative(SB, 0x40), 0);
        assert_eq!(decode_relative(SB, 0x00), 0);
    }

    #[test]
    fn high_bit_is_masked_off() {
        // Status/value bytes are 7-bit; a stray bit 7 must not change the result.
        assert_eq!(
            decode_relative(RelEncoding::BinaryOffset, 0xC1),
            decode_relative(RelEncoding::BinaryOffset, 0x41),
        );
    }
}
