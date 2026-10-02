//! The CABAC arithmetic decoder and the context models of intra slices.

/// Probability state of one context: a 6-bit state and the most probable symbol.
#[derive(Clone, Copy, Default)]
pub struct Context {
    state: u8,
    mps: u8,
}

const LPS_RANGE: [[u8; 4]; 64] = [
    [128, 176, 208, 240],
    [128, 167, 197, 227],
    [128, 158, 187, 216],
    [123, 150, 178, 205],
    [116, 142, 169, 195],
    [111, 135, 160, 185],
    [105, 128, 152, 175],
    [100, 122, 144, 166],
    [95, 116, 137, 158],
    [90, 110, 130, 150],
    [85, 104, 123, 142],
    [81, 99, 117, 135],
    [77, 94, 111, 128],
    [73, 89, 105, 122],
    [69, 85, 100, 116],
    [66, 80, 95, 110],
    [62, 76, 90, 104],
    [59, 72, 86, 99],
    [56, 69, 81, 94],
    [53, 65, 77, 89],
    [51, 62, 73, 85],
    [48, 59, 69, 80],
    [46, 56, 66, 76],
    [43, 53, 63, 72],
    [41, 50, 59, 69],
    [39, 48, 56, 65],
    [37, 45, 54, 62],
    [35, 43, 51, 59],
    [33, 41, 48, 56],
    [32, 39, 46, 53],
    [30, 37, 43, 50],
    [29, 35, 41, 48],
    [27, 33, 39, 45],
    [26, 31, 37, 43],
    [24, 30, 35, 41],
    [23, 28, 33, 39],
    [22, 27, 32, 37],
    [21, 26, 30, 35],
    [20, 24, 29, 33],
    [19, 23, 27, 31],
    [18, 22, 26, 30],
    [17, 21, 25, 28],
    [16, 20, 23, 27],
    [15, 19, 22, 25],
    [14, 18, 21, 24],
    [14, 17, 20, 23],
    [13, 16, 19, 22],
    [12, 15, 18, 21],
    [12, 14, 17, 20],
    [11, 14, 16, 19],
    [11, 13, 15, 18],
    [10, 12, 15, 17],
    [10, 12, 14, 16],
    [9, 11, 13, 15],
    [9, 11, 12, 14],
    [8, 10, 12, 14],
    [8, 9, 11, 13],
    [7, 9, 11, 12],
    [7, 9, 10, 12],
    [7, 8, 10, 11],
    [6, 8, 9, 11],
    [6, 7, 9, 10],
    [6, 7, 8, 9],
    [2, 2, 2, 2],
];
const NEXT_MPS: [u8; 64] = [
    1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20, 21, 22, 23, 24, 25, 26,
    27, 28, 29, 30, 31, 32, 33, 34, 35, 36, 37, 38, 39, 40, 41, 42, 43, 44, 45, 46, 47, 48, 49, 50,
    51, 52, 53, 54, 55, 56, 57, 58, 59, 60, 61, 62, 62, 63,
];
const NEXT_LPS: [u8; 64] = [
    0, 0, 1, 2, 2, 4, 4, 5, 6, 7, 8, 9, 9, 11, 11, 12, 13, 13, 15, 15, 16, 16, 18, 18, 19, 19, 21,
    21, 22, 22, 23, 24, 24, 25, 26, 26, 27, 27, 28, 29, 29, 30, 30, 30, 31, 32, 32, 33, 33, 33, 34,
    34, 35, 35, 35, 36, 36, 36, 37, 37, 37, 38, 38, 63,
];
/// Bits to shift out after an LPS, by the LPS range divided by 8.
const RENORM: [u8; 32] = [
    6, 5, 4, 4, 3, 3, 3, 3, 2, 2, 2, 2, 2, 2, 2, 2, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1,
];

/// Offsets of the context models, in the order of the initial values below.
pub mod ctx {
    pub const SAO_MERGE: usize = 0;
    pub const SAO_TYPE: usize = 1;
    pub const SPLIT_CU: usize = 2; // 3
    pub const TRANSQUANT_BYPASS: usize = 5;
    pub const CU_QP_DELTA: usize = 6; // 3
    pub const PART_MODE: usize = 9;
    pub const PREV_INTRA_LUMA: usize = 10;
    pub const INTRA_CHROMA: usize = 11;
    pub const SPLIT_TRANSFORM: usize = 12; // 3
    pub const CBF_LUMA: usize = 15; // 2
    pub const CBF_CHROMA: usize = 17; // 5
    pub const TRANSFORM_SKIP: usize = 22; // 2: luma, chroma
    pub const LAST_X_PREFIX: usize = 24; // 18
    pub const LAST_Y_PREFIX: usize = 42; // 18
    pub const CODED_SUB_BLOCK: usize = 60; // 4
    pub const SIG_COEFF: usize = 64; // 44
    pub const GREATER1: usize = 108; // 24
    pub const GREATER2: usize = 132; // 6
    pub const CU_CHROMA_QP_OFFSET_FLAG: usize = 138;
    pub const CU_CHROMA_QP_OFFSET_IDX: usize = 139;
    // 140..150 belong to cross-component prediction, which is not supported;
    // they keep the table in the order of the standard.
    pub const COUNT: usize = 150;
}

/// Initial values for slices of type I (initType 0), in the order of `ctx`.
#[rustfmt::skip]
const INIT_I: [u8; ctx::COUNT] = [
    153,                               // sao_merge_flag
    200,                               // sao_type_idx
    139, 141, 157,                     // split_cu_flag
    154,                               // cu_transquant_bypass_flag
    154, 154, 154,                     // cu_qp_delta_abs
    184,                               // part_mode
    184,                               // prev_intra_luma_pred_flag
    63,                                // intra_chroma_pred_mode
    153, 138, 138,                     // split_transform_flag
    111, 141,                          // cbf_luma
    94, 138, 182, 154, 154,            // cbf_cb, cbf_cr
    139, 139,                          // transform_skip_flag
    110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63, // last_x_prefix
    110, 110, 124, 125, 140, 153, 125, 127, 140, 109, 111, 143, 127, 111, 79, 108, 123, 63, // last_y_prefix
    91, 171, 134, 141,                 // coded_sub_block_flag
    111, 111, 125, 110, 110, 94, 124, 108, 124, 107, 125, 141, 179, 153,
    125, 107, 125, 141, 179, 153, 125, 107, 125, 141, 179, 153, 125, 140,
    139, 182, 182, 152, 136, 152, 136, 153, 136, 139, 111, 136, 139, 111,
    141, 111,                          // sig_coeff_flag
    140, 92, 137, 138, 140, 152, 138, 139, 153, 74, 149, 92, 139, 107,
    122, 152, 140, 179, 166, 182, 140, 227, 122, 197, // coeff_abs_level_greater1_flag
    138, 153, 136, 167, 152, 152,      // coeff_abs_level_greater2_flag
    154,                               // cu_chroma_qp_offset_flag
    154,                               // cu_chroma_qp_offset_idx
    154, 154, 154, 154, 154, 154, 154, 154, // log2_res_scale_abs_plus1
    154, 154,                          // res_scale_sign_flag
];

#[derive(Clone)]
pub struct Models(pub [Context; ctx::COUNT]);
impl Models {
    /// Initial states for a slice with quantization parameter `qp`.
    pub fn new(qp: i32) -> Self {
        let qp = qp.clamp(0, 51);
        let mut models = [Context::default(); ctx::COUNT];
        for (model, &value) in models.iter_mut().zip(&INIT_I) {
            let slope = i32::from(value >> 4) * 5 - 45;
            let offset = (i32::from(value & 15) << 3) - 16;
            let pre = (((slope * qp) >> 4) + offset).clamp(1, 126);
            *model = if pre <= 63 {
                Context {
                    state: (63 - pre) as u8,
                    mps: 0,
                }
            } else {
                Context {
                    state: (pre - 64) as u8,
                    mps: 1,
                }
            };
        }
        Self(models)
    }
}

/// The arithmetic decoder over the bytes of one slice segment.
pub struct Cabac<'a> {
    data: &'a [u8],
    position: usize,
    range: u32,
    value: u32,
    bits_needed: i32,
}

impl<'a> Cabac<'a> {
    /// Starts decoding at `position`.
    pub fn new(data: &'a [u8], position: usize) -> Self {
        let mut cabac = Self {
            data,
            position,
            range: 510,
            value: 0,
            bits_needed: 8,
        };
        cabac.start();
        cabac
    }
    /// (Re)initializes the engine at the current byte position, as at the start
    /// of a slice segment, a new substream or after PCM samples.
    pub fn start(&mut self) {
        self.range = 510;
        self.bits_needed = 8;
        self.value = 0;
        if let Some(&byte) = self.data.get(self.position) {
            self.value = u32::from(byte) << 8;
            self.position += 1;
            self.bits_needed -= 8;
        }
        if let Some(&byte) = self.data.get(self.position) {
            self.value |= u32::from(byte);
            self.position += 1;
            self.bits_needed -= 8;
        }
    }
    /// Offset of the next unread byte. After a terminating bin this is where
    /// the next substream or the PCM samples begin.
    pub fn byte_position(&self) -> usize {
        self.position
    }
    /// The bytes from the current position on.
    pub fn remaining(&self) -> &'a [u8] {
        self.data.get(self.position..).unwrap_or(&[])
    }
    pub fn set_byte_position(&mut self, position: usize) {
        self.position = position;
    }
    /// True once the decoder has read well beyond the data, which only happens
    /// for damaged streams.
    pub fn overrun(&self) -> bool {
        self.position > self.data.len() + 8
    }

    fn next_byte(&mut self) -> u32 {
        let byte = self.data.get(self.position).copied().unwrap_or(0);
        self.position += 1;
        u32::from(byte)
    }

    pub fn bin(&mut self, model: &mut Context) -> u32 {
        let lps = u32::from(LPS_RANGE[model.state as usize][((self.range >> 6) - 4) as usize]);
        self.range -= lps;
        let scaled = self.range << 7;
        if self.value < scaled {
            let bit = u32::from(model.mps);
            model.state = NEXT_MPS[model.state as usize];
            if scaled < (256 << 7) {
                self.range = scaled >> 6;
                self.value <<= 1;
                self.bits_needed += 1;
                if self.bits_needed == 0 {
                    self.bits_needed = -8;
                    self.value |= self.next_byte();
                }
            }
            bit
        } else {
            self.value -= scaled;
            let shift = RENORM[(lps >> 3) as usize];
            self.value <<= shift;
            self.range = lps << shift;
            let bit = 1 - u32::from(model.mps);
            if model.state == 0 {
                model.mps = 1 - model.mps;
            }
            model.state = NEXT_LPS[model.state as usize];
            self.bits_needed += i32::from(shift);
            if self.bits_needed >= 0 {
                let byte = self.next_byte();
                self.value |= byte << self.bits_needed;
                self.bits_needed -= 8;
            }
            bit
        }
    }

    pub fn bypass(&mut self) -> u32 {
        self.value <<= 1;
        self.bits_needed += 1;
        if self.bits_needed >= 0 {
            self.bits_needed = -8;
            self.value |= self.next_byte();
        }
        let scaled = self.range << 7;
        if self.value >= scaled {
            self.value -= scaled;
            1
        } else {
            0
        }
    }
    /// `count` bypass bins as an unsigned number, most significant first.
    pub fn bypass_bits(&mut self, count: u32) -> u32 {
        let mut value = 0;
        for _ in 0..count {
            value = value << 1 | self.bypass();
        }
        value
    }

    /// The terminating bin (end of slice segment, end of substream, PCM).
    pub fn terminate(&mut self) -> bool {
        self.range -= 2;
        let scaled = self.range << 7;
        if self.value >= scaled {
            return true;
        }
        if scaled < (256 << 7) {
            self.range = scaled >> 6;
            self.value <<= 1;
            self.bits_needed += 1;
            if self.bits_needed == 0 {
                self.bits_needed = -8;
                self.value += self.next_byte();
            }
        }
        false
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn context_initialization_follows_the_formula() {
        // initValue 154 (slope index 9, offset index 10): m = 0 and n = 64, so
        // preCtxState is 64: the most probable symbol is 1, in state 0.
        let models = Models::new(26);
        let neutral = models.0[ctx::TRANSQUANT_BYPASS];
        assert_eq!((neutral.state, neutral.mps), (0, 1));
        // 153 has offset index 9, so n = 56: symbol 0 in state 7.
        let merge = models.0[ctx::SAO_MERGE];
        assert_eq!((merge.state, merge.mps), (7, 0));
        // The QP is clamped to the table's range.
        assert_eq!(Models::new(-5).0[0].state, Models::new(0).0[0].state);
        assert_eq!(Models::new(60).0[0].state, Models::new(51).0[0].state);
    }
    #[test]
    fn bypass_bins_of_zero_bytes_are_zero() {
        let data = [0u8; 8];
        let mut cabac = Cabac::new(&data, 0);
        assert_eq!(cabac.bypass_bits(16), 0);
    }
}
