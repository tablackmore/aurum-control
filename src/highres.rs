//! 14-bit (high-resolution) Control Change reassembly.
//!
//! Per the MIDI spec, controllers 0..31 are the MSB of a 14-bit value whose LSB
//! is sent on controller (n + 32). This decoder tracks the latest MSB per
//! channel/controller and combines it with the LSB, yielding a *logical*
//! controller number (the MSB index) and a normalized 0..1 value. Controllers
//! >= 64 are treated as plain 7-bit.
//!
//! Whether a controller in 0..31 is half of a 14-bit pair or a plain 7-bit knob
//! is learned from traffic: once an LSB (n + 32) is seen, the pair is marked
//! hi-res and later MSB messages yield `None` — the LSB that follows carries
//! the full-resolution value. This keeps plain 7-bit controllers in 0..31
//! working while stopping a hi-res knob from dispatching twice (coarse then
//! fine) per movement. Only the pair's very first movement emits both.

#[derive(Debug)]
pub struct HighResDecoder {
    msb: [[u8; 32]; 16], // [channel][controller 0..31]
    hires: [u32; 16],    // per channel: bit n set = LSB seen for controller n
}

impl Default for HighResDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HighResDecoder {
    pub fn new() -> Self {
        Self {
            msb: [[0; 32]; 16],
            hires: [0; 16],
        }
    }

    /// Feed a CC. Returns `Some((logical_controller, value 0..1))`, or `None`
    /// for the MSB of a known hi-res pair (its LSB carries the value).
    pub fn feed(&mut self, channel: u8, controller: u8, value: u8) -> Option<(u8, f32)> {
        let ch = (channel & 0x0F) as usize;
        let v = value & 0x7F;
        if controller < 32 {
            self.msb[ch][controller as usize] = v;
            if self.hires[ch] & (1 << controller) != 0 {
                return None;
            }
            Some((controller, v as f32 / 127.0))
        } else if controller < 64 {
            let i = (controller - 32) as usize;
            self.hires[ch] |= 1 << i;
            let v14 = ((self.msb[ch][i] as u16) << 7) | v as u16;
            Some((controller - 32, v14 as f32 / 16383.0))
        } else {
            Some((controller, v as f32 / 127.0))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_7bit_controller() {
        let mut d = HighResDecoder::new();
        let (logical, v) = d.feed(0, 74, 127).unwrap();
        assert_eq!(logical, 74);
        assert!((v - 1.0).abs() < 1e-6);
    }

    #[test]
    fn msb_then_lsb_combine_to_14bit() {
        let mut d = HighResDecoder::new();
        let (lm, _) = d.feed(0, 10, 100).unwrap();
        assert_eq!(lm, 10);
        let (logical, v) = d.feed(0, 42, 50).unwrap();
        assert_eq!(logical, 10, "LSB resolves to the same logical controller");
        let expected = (((100u16) << 7) | 50) as f32 / 16383.0;
        assert!((v - expected).abs() < 1e-6);
    }

    #[test]
    fn msb_only_is_coarse() {
        let mut d = HighResDecoder::new();
        let (_, v) = d.feed(0, 0, 64).unwrap();
        assert!((v - 64.0 / 127.0).abs() < 1e-6);
    }

    #[test]
    fn msb_suppressed_once_pair_is_known_hires() {
        let mut d = HighResDecoder::new();
        // First movement: pair not yet known — MSB emits coarse, LSB refines.
        assert!(d.feed(0, 10, 100).is_some());
        assert!(d.feed(0, 42, 50).is_some());
        // Later movements: only the LSB emits (full 14-bit value).
        assert!(d.feed(0, 10, 101).is_none());
        let (logical, v) = d.feed(0, 42, 60).unwrap();
        assert_eq!(logical, 10);
        let expected = (((101u16) << 7) | 60) as f32 / 16383.0;
        assert!((v - expected).abs() < 1e-6);
        // Hi-res learning is per channel: the same controller on another
        // channel still emits its MSB.
        assert!(d.feed(1, 10, 100).is_some());
    }
}
