//! 14-bit (high-resolution) Control Change reassembly.
//!
//! Per the MIDI spec, controllers 0..31 are the MSB of a 14-bit value whose LSB
//! is sent on controller (n + 32). This decoder tracks the latest MSB per
//! channel/controller and combines it with the LSB, yielding a *logical*
//! controller number (the MSB index) and a normalized 0..1 value. Controllers
//! >= 64 are treated as plain 7-bit.

#[derive(Debug)]
pub struct HighResDecoder {
    msb: [[u8; 32]; 16], // [channel][controller 0..31]
}

impl Default for HighResDecoder {
    fn default() -> Self {
        Self::new()
    }
}

impl HighResDecoder {
    pub fn new() -> Self {
        Self { msb: [[0; 32]; 16] }
    }

    /// Feed a CC. Returns `(logical_controller, value 0..1)`.
    pub fn feed(&mut self, channel: u8, controller: u8, value: u8) -> (u8, f32) {
        let ch = (channel & 0x0F) as usize;
        let v = value & 0x7F;
        if controller < 32 {
            self.msb[ch][controller as usize] = v;
            (controller, v as f32 / 127.0)
        } else if controller < 64 {
            let i = (controller - 32) as usize;
            let v14 = ((self.msb[ch][i] as u16) << 7) | v as u16;
            (controller - 32, v14 as f32 / 16383.0)
        } else {
            (controller, v as f32 / 127.0)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_7bit_controller() {
        let mut d = HighResDecoder::new();
        let (logical, v) = d.feed(0, 74, 127);
        assert_eq!(logical, 74);
        assert!((v - 1.0).abs() < 1e-6);
    }

    #[test]
    fn msb_then_lsb_combine_to_14bit() {
        let mut d = HighResDecoder::new();
        let (lm, _) = d.feed(0, 10, 100);
        assert_eq!(lm, 10);
        let (logical, v) = d.feed(0, 42, 50);
        assert_eq!(logical, 10, "LSB resolves to the same logical controller");
        let expected = (((100u16) << 7) | 50) as f32 / 16383.0;
        assert!((v - expected).abs() < 1e-6);
    }

    #[test]
    fn msb_only_is_coarse() {
        let mut d = HighResDecoder::new();
        let (_, v) = d.feed(0, 0, 64);
        assert!((v - 64.0 / 127.0).abs() < 1e-6);
    }
}
