//! The picture being decoded: its samples and the block information that the
//! prediction and the in-loop filters need.
use super::params::{Pps, Sps};

/// Picture-wide information is kept per 4x4 luma block, the smallest unit
/// (the minimum transform block) of any coding structure.
pub const FLAG_BYPASS: u8 = 1; // cu_transquant_bypass_flag
pub const FLAG_PCM_NO_FILTER: u8 = 2; // PCM with the loop filter disabled
pub const EDGE_VERTICAL: u8 = 1; // a transform/coding block edge on the left
pub const EDGE_HORIZONTAL: u8 = 2; // ... on the top

#[derive(Clone, Copy, Default, Debug)]
pub struct SaoParams {
    /// 0 off, 1 band offset, 2 edge offset.
    pub kind: u8,
    pub band_position: u8,
    pub eo_class: u8,
    /// Offsets for categories 1 to 4, already scaled.
    pub offsets: [i16; 4],
}

/// What the filters need to know about the slice that a CTB belongs to.
#[derive(Clone, Debug)]
pub struct SliceInfo {
    /// Address of the first CTB of the slice (not the segment).
    pub address: u32,
    pub deblocking_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub loop_filter_across_slices: bool,
}

pub struct Picture<'a> {
    pub sps: &'a Sps,
    pub pps: &'a Pps,
    pub planes: [Vec<u16>; 3],
    pub plane_width: [usize; 3],
    pub plane_height: [usize; 3],
    pub w4: usize,
    pub decoded: Vec<bool>,
    pub intra_mode: Vec<u8>,
    pub cu_depth: Vec<u8>,
    pub qp_y: Vec<i8>,
    pub flags: Vec<u8>,
    pub edges: Vec<u8>,
    pub ctb_w: usize,
    pub ctb_h: usize,
    /// Index into `slices` for each CTB, or -1 for a CTB not decoded (yet).
    pub ctb_slice: Vec<i32>,
    pub slices: Vec<SliceInfo>,
    pub sao: Vec<[SaoParams; 3]>,
}

impl<'a> Picture<'a> {
    pub fn new(sps: &'a Sps, pps: &'a Pps) -> Self {
        let (w, h) = (sps.width as usize, sps.height as usize);
        let chroma = sps.chroma_array_type != 0;
        let cw = if chroma {
            w.div_ceil(sps.sub_width() as usize)
        } else {
            0
        };
        let ch = if chroma {
            h.div_ceil(sps.sub_height() as usize)
        } else {
            0
        };
        let (w4, h4) = (w.div_ceil(4), h.div_ceil(4));
        let (ctb_w, ctb_h) = (sps.width_in_ctbs() as usize, sps.height_in_ctbs() as usize);
        // Mid-grey where nothing is decoded, so damage shows as grey not as noise.
        let grey = |depth: u8| 1u16 << (depth - 1);
        Self {
            sps,
            pps,
            planes: [
                vec![grey(sps.bit_depth_luma); w * h],
                vec![grey(sps.bit_depth_chroma); cw * ch],
                vec![grey(sps.bit_depth_chroma); cw * ch],
            ],
            plane_width: [w, cw, cw],
            plane_height: [h, ch, ch],
            w4,
            decoded: vec![false; w4 * h4],
            intra_mode: vec![1; w4 * h4],
            cu_depth: vec![0; w4 * h4],
            qp_y: vec![0; w4 * h4],
            flags: vec![0; w4 * h4],
            edges: vec![0; w4 * h4],
            ctb_w,
            ctb_h,
            ctb_slice: vec![-1; ctb_w * ctb_h],
            slices: Vec::new(),
            sao: vec![[SaoParams::default(); 3]; ctb_w * ctb_h],
        }
    }
    pub fn at4(&self, x: usize, y: usize) -> usize {
        (y >> 2) * self.w4 + (x >> 2)
    }
    pub fn ctb_of(&self, x: usize, y: usize) -> usize {
        (y >> self.sps.log2_ctb) * self.ctb_w + (x >> self.sps.log2_ctb)
    }
    pub fn slice_address_at(&self, x: usize, y: usize) -> Option<u32> {
        let index = self.ctb_slice[self.ctb_of(x, y)];
        usize::try_from(index).ok().map(|i| self.slices[i].address)
    }
    /// Whether the luma position holds a decoded block of the same slice as the
    /// one being decoded (6.4.1).
    pub fn available(&self, slice_address: u32, x: i64, y: i64) -> bool {
        if x < 0 || y < 0 || x >= i64::from(self.sps.width) || y >= i64::from(self.sps.height) {
            return false;
        }
        let (x, y) = (x as usize, y as usize);
        self.decoded[self.at4(x, y)] && self.slice_address_at(x, y) == Some(slice_address)
    }
    /// Fills a rectangle of a per-4x4 map.
    pub fn fill<T: Copy>(
        map: &mut [T],
        w4: usize,
        x: usize,
        y: usize,
        w: usize,
        h: usize,
        value: T,
    ) {
        let (x4, y4) = (x >> 2, y >> 2);
        let (w4s, h4s) = (w.div_ceil(4).max(1), h.div_ceil(4).max(1));
        let rows = map.len() / w4;
        for row in y4..(y4 + h4s).min(rows) {
            let start = row * w4 + x4;
            let end = (row * w4 + x4 + w4s).min((row + 1) * w4);
            map[start..end].fill(value);
        }
    }
}
