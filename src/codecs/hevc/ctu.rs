//! Slice segment data of intra pictures: the coding tree units, their coding
//! quadtrees, coding units, transform trees and residuals (7.3.8), each
//! reconstructed as soon as it is parsed.
use super::{
    bits::{Bits, bad},
    cabac::{Cabac, Models, ctx},
    intra::MODE_422,
    params::{Pps, ScalingLists, Sps, diagonal_scan},
    picture::{
        EDGE_HORIZONTAL, EDGE_VERTICAL, FLAG_BYPASS, FLAG_PCM_NO_FILTER, Picture, SaoParams,
    },
    slice::SliceHeader,
    transform::{self, LEVEL_SCALE},
};
use crate::{error::Error, security::Ticket};
use std::sync::OnceLock;

/// State handed from one slice segment to the next, and from one CTB row to the
/// next with wavefront parallel processing.
#[derive(Clone)]
pub struct Carry {
    pub models: Models,
    pub stat_coeff: [u8; 4],
    pub last_qp_y: i32,
}

#[derive(Default)]
pub struct Stores {
    /// Saved after the second CTB of a row, for the row below.
    pub wavefront: Option<Carry>,
    /// Saved at the end of a segment, for a dependent segment that may follow.
    pub segment: Option<Carry>,
}

type Scan = Vec<(u8, u8)>;

/// Scan orders: `[log2 size][scan index]`, for block sizes 1, 2, 4 and 8, where
/// the scan index is 0 diagonal, 1 horizontal, 2 vertical.
fn scans() -> &'static [[Scan; 3]; 4] {
    static SCANS: OnceLock<[[Scan; 3]; 4]> = OnceLock::new();
    SCANS.get_or_init(|| {
        std::array::from_fn(|log2| {
            let n = 1usize << log2;
            let horizontal: Scan = (0..n)
                .flat_map(|y| (0..n).map(move |x| (x as u8, y as u8)))
                .collect();
            let vertical: Scan = (0..n)
                .flat_map(|x| (0..n).map(move |y| (x as u8, y as u8)))
                .collect();
            [diagonal_scan(n), horizontal, vertical]
        })
    })
}

/// Context increments of `sig_coeff_flag` for 4x4 blocks, by position.
const SIG_CTX_4X4: [u8; 16] = [0, 1, 4, 5, 2, 3, 4, 5, 6, 6, 8, 8, 7, 7, 8, 8];

/// Chroma QP for luma-derived values from 30 to 43 (4:2:0).
const CHROMA_QP: [i32; 14] = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37];

pub struct Segment<'s, 'p, 'd> {
    pub pic: &'s mut Picture<'p>,
    cabac: Cabac<'d>,
    models: Models,
    header: &'s SliceHeader,
    ticket: &'s Ticket,
    slice_index: i32,
    slice_address: u32,
    sps: &'p Sps,
    pps: &'p Pps,
    /// `QpY` of the coding unit being decoded.
    qp_y: i32,
    /// `qPY_PRED` of the quantization group.
    qp_y_pred: i32,
    /// `QpY` of the last coding unit of the previous quantization group.
    last_qp_y: i32,
    cu_qp_delta_coded: bool,
    cu_chroma_qp_offset_coded: bool,
    cu_qp_offset: [i32; 2],
    bypass: bool,
    stat_coeff: [u8; 4],
    coefficients: Vec<i32>,
    residual: Vec<i32>,
    chroma_modes: [u8; 4],
    cu: (usize, usize, u32),
    intra_split: bool,
    scaling: Option<&'p ScalingLists>,
}

/// What `residual_coding` found.
struct Block {
    transform_skip: bool,
    max_x: usize,
    max_y: usize,
}

impl<'s, 'p, 'd> Segment<'s, 'p, 'd> {
    pub fn new(
        pic: &'s mut Picture<'p>,
        header: &'s SliceHeader,
        slice_index: i32,
        slice_address: u32,
        data: &'d [u8],
        carry: Option<Carry>,
        ticket: &'s Ticket,
    ) -> Self {
        let (sps, pps) = (pic.sps, pic.pps);
        let carry = carry.unwrap_or_else(|| Carry {
            models: Models::new(header.qp),
            stat_coeff: [0; 4],
            last_qp_y: header.qp,
        });
        let scaling = pps.scaling_lists.as_ref().or(sps.scaling_lists.as_ref());
        Self {
            pic,
            cabac: Cabac::new(data, header.data_offset),
            models: carry.models,
            header,
            ticket,
            slice_index,
            slice_address,
            sps,
            pps,
            qp_y: header.qp,
            qp_y_pred: header.qp,
            last_qp_y: carry.last_qp_y,
            cu_qp_delta_coded: false,
            cu_chroma_qp_offset_coded: false,
            cu_qp_offset: [0, 0],
            bypass: false,
            stat_coeff: carry.stat_coeff,
            coefficients: vec![0; 32 * 32],
            residual: vec![0; 32 * 32],
            chroma_modes: [1; 4],
            cu: (0, 0, 3),
            intra_split: false,
            scaling,
        }
    }

    fn carry(&self) -> Carry {
        Carry {
            models: self.models.clone(),
            stat_coeff: self.stat_coeff,
            last_qp_y: self.last_qp_y,
        }
    }

    /// Decodes the CTBs of the segment, from its first address to its end flag.
    pub fn decode(&mut self, stores: &mut Stores) -> Result<(), Error> {
        let (ctb_w, ctb_h) = (self.pic.ctb_w, self.pic.ctb_h);
        let total = ctb_w * ctb_h;
        let mut address = self.header.segment_address as usize;
        let mut first = true;
        loop {
            let (cx, cy) = (address % ctb_w, address / ctb_w);
            // The slice must own the CTB before anything is asked about its slice.
            self.pic.ctb_slice[address] = self.slice_index;
            if self.pps.entropy_coding_sync
                && cx == 0
                && cy >= 1
                && (!first || self.header.dependent)
            {
                // Wavefront: continue from the row above if its second CTB is
                // part of this slice, otherwise start afresh.
                let ctb = 1usize << self.sps.log2_ctb;
                let above = ctb_w > 1
                    && self
                        .pic
                        .available(self.slice_address, ctb as i64, ((cy - 1) * ctb) as i64);
                match (&stores.wavefront, above) {
                    (Some(carry), true) => {
                        self.models = carry.models.clone();
                        self.stat_coeff = carry.stat_coeff;
                    }
                    _ => {
                        self.models = Models::new(self.header.qp);
                        self.stat_coeff = [0; 4];
                    }
                }
                self.last_qp_y = self.header.qp;
            }
            first = false;
            if cx == 0 {
                self.ticket.check()?;
            }
            self.ctu(cx, cy)?;
            if self.pps.entropy_coding_sync && cx == 1 && cy + 1 < ctb_h {
                stores.wavefront = Some(self.carry());
            }
            if self.cabac.terminate() {
                if self.pps.dependent_slice_segments {
                    stores.segment = Some(self.carry());
                }
                return Ok(());
            }
            if self.cabac.overrun() {
                return Err(bad("the slice data ends early"));
            }
            address += 1;
            if address >= total {
                return Err(bad("the slice continues past the last CTB"));
            }
            if self.pps.entropy_coding_sync && address.is_multiple_of(ctb_w) {
                // End of a substream: a terminating bin, then a fresh engine at
                // the next byte.
                if !self.cabac.terminate() {
                    return Err(bad("a substream does not end where it should"));
                }
                self.cabac.start();
            }
        }
    }

    fn ctu(&mut self, cx: usize, cy: usize) -> Result<(), Error> {
        let log2_ctb = u32::from(self.sps.log2_ctb);
        if self.header.sao_luma || self.header.sao_chroma {
            self.sao(cx, cy);
        } else {
            self.pic.sao[cy * self.pic.ctb_w + cx] = [SaoParams::default(); 3];
        }
        self.coding_quadtree(cx << log2_ctb, cy << log2_ctb, log2_ctb, 0)
    }

    // --- sample adaptive offset parameters (7.3.8.3) ---

    fn sao(&mut self, cx: usize, cy: usize) {
        let ctb_w = self.pic.ctb_w;
        let address = cy * ctb_w + cx;
        let slice = self.slice_address as usize;
        if cx > 0 && address > slice && self.cabac.bin(&mut self.models.0[ctx::SAO_MERGE]) == 1 {
            self.pic.sao[address] = self.pic.sao[address - 1];
            return;
        }
        if cy > 0
            && address >= slice + ctb_w
            && self.cabac.bin(&mut self.models.0[ctx::SAO_MERGE]) == 1
        {
            self.pic.sao[address] = self.pic.sao[address - ctb_w];
            return;
        }
        let mut params = [SaoParams::default(); 3];
        let components = if self.sps.chroma_array_type != 0 {
            3
        } else {
            1
        };
        for c in 0..components {
            if (c == 0 && !self.header.sao_luma) || (c > 0 && !self.header.sao_chroma) {
                continue;
            }
            let kind = if c == 2 {
                params[1].kind
            } else if self.cabac.bin(&mut self.models.0[ctx::SAO_TYPE]) == 0 {
                0
            } else {
                1 + self.cabac.bypass() as u8
            };
            params[c].kind = kind;
            if kind == 0 {
                continue;
            }
            let depth = if c == 0 {
                self.sps.bit_depth_luma
            } else {
                self.sps.bit_depth_chroma
            };
            let max = (1u32 << (depth.min(10) - 5)) - 1;
            let mut offsets = [0i32; 4];
            for offset in &mut offsets {
                let mut value = 0;
                while value < max && self.cabac.bypass() == 1 {
                    value += 1;
                }
                *offset = value as i32;
            }
            if kind == 1 {
                for offset in &mut offsets {
                    if *offset != 0 && self.cabac.bypass() == 1 {
                        *offset = -*offset;
                    }
                }
                params[c].band_position = self.cabac.bypass_bits(5) as u8;
            } else {
                offsets[2] = -offsets[2];
                offsets[3] = -offsets[3];
                params[c].eo_class = if c == 2 {
                    params[1].eo_class
                } else {
                    self.cabac.bypass_bits(2) as u8
                };
            }
            let scale = if c == 0 {
                self.pps.log2_sao_offset_scale_luma
            } else {
                self.pps.log2_sao_offset_scale_chroma
            };
            for (stored, offset) in params[c].offsets.iter_mut().zip(offsets) {
                *stored = (offset << scale) as i16;
            }
        }
        self.pic.sao[address] = params;
    }

    // --- coding quadtree (7.3.8.4) ---

    fn coding_quadtree(
        &mut self,
        x0: usize,
        y0: usize,
        log2_cb: u32,
        depth: u8,
    ) -> Result<(), Error> {
        let size = 1usize << log2_cb;
        let sps = self.sps;
        let split = if x0 + size <= sps.width as usize
            && y0 + size <= sps.height as usize
            && log2_cb > u32::from(sps.log2_min_cb)
        {
            self.split_cu_flag(x0, y0, depth)
        } else {
            log2_cb > u32::from(sps.log2_min_cb)
        };
        let log2_ctb = u32::from(sps.log2_ctb);
        if self.pps.cu_qp_delta && log2_cb + self.pps.diff_cu_qp_delta_depth >= log2_ctb {
            self.cu_qp_delta_coded = false;
            self.start_quant_group(x0, y0);
        }
        if self.header.cu_chroma_qp_offset_enabled
            && log2_cb + self.pps.diff_cu_chroma_qp_offset_depth >= log2_ctb
        {
            self.cu_chroma_qp_offset_coded = false;
        }
        if split {
            let half = size / 2;
            for (dx, dy) in [(0, 0), (half, 0), (0, half), (half, half)] {
                if x0 + dx < sps.width as usize && y0 + dy < sps.height as usize {
                    self.coding_quadtree(x0 + dx, y0 + dy, log2_cb - 1, depth + 1)?;
                }
            }
            Ok(())
        } else {
            self.coding_unit(x0, y0, log2_cb, depth)
        }
    }

    fn split_cu_flag(&mut self, x0: usize, y0: usize, depth: u8) -> bool {
        let mut increment = 0;
        let (x, y) = (x0 as i64, y0 as i64);
        if self.pic.available(self.slice_address, x - 1, y)
            && self.pic.cu_depth[self.pic.at4(x0 - 1, y0)] > depth
        {
            increment += 1;
        }
        if self.pic.available(self.slice_address, x, y - 1)
            && self.pic.cu_depth[self.pic.at4(x0, y0 - 1)] > depth
        {
            increment += 1;
        }
        self.cabac
            .bin(&mut self.models.0[ctx::SPLIT_CU + increment])
            == 1
    }

    // --- quantization parameters (8.6.1) ---

    /// Begins a quantization group at (`x0`, `y0`): predicts its QP from the
    /// groups to the left and above, when they are in the same CTB, and else
    /// from the previous group in decoding order.
    fn start_quant_group(&mut self, x0: usize, y0: usize) {
        let ctb = (1usize << self.sps.log2_ctb) - 1;
        let left = if x0 & ctb != 0 {
            i32::from(self.pic.qp_y[self.pic.at4(x0 - 1, y0)])
        } else {
            self.last_qp_y
        };
        let above = if y0 & ctb != 0 {
            i32::from(self.pic.qp_y[self.pic.at4(x0, y0 - 1)])
        } else {
            self.last_qp_y
        };
        self.qp_y_pred = (left + above + 1) >> 1;
        self.qp_y = self.qp_y_pred;
    }

    /// Chroma QP (`Qp'Cb` or `Qp'Cr`) from the luma QP.
    fn chroma_qp(&self, c: usize) -> i32 {
        let (pps_offset, slice_offset) = if c == 1 {
            (self.pps.cb_qp_offset, self.header.cb_qp_offset)
        } else {
            (self.pps.cr_qp_offset, self.header.cr_qp_offset)
        };
        let bd = self.sps.qp_bd_offset_chroma();
        let qpi = (self.qp_y + pps_offset + slice_offset + self.cu_qp_offset[c - 1]).clamp(-bd, 57);
        let qpc = if self.sps.chroma_array_type == 1 {
            match qpi {
                ..30 => qpi,
                30..=43 => CHROMA_QP[(qpi - 30) as usize],
                _ => qpi - 6,
            }
        } else {
            qpi.min(51)
        };
        qpc + bd
    }

    // --- coding unit (7.3.8.5) ---

    fn coding_unit(&mut self, x0: usize, y0: usize, log2_cb: u32, depth: u8) -> Result<(), Error> {
        let size = 1usize << log2_cb;
        let sps = self.sps;
        let w4 = self.pic.w4;
        self.cu = (x0, y0, log2_cb);
        self.bypass = self.pps.transquant_bypass
            && self.cabac.bin(&mut self.models.0[ctx::TRANSQUANT_BYPASS]) == 1;
        // In an intra slice the partition is the only thing left to say.
        let nxn = log2_cb == u32::from(sps.log2_min_cb)
            && self.cabac.bin(&mut self.models.0[ctx::PART_MODE]) == 0;
        self.intra_split = nxn;
        Picture::fill(&mut self.pic.cu_depth, w4, x0, y0, size, size, depth);
        Picture::fill(
            &mut self.pic.flags,
            w4,
            x0,
            y0,
            size,
            size,
            if self.bypass { FLAG_BYPASS } else { 0 },
        );
        let pcm =
            !nxn && sps.pcm.as_ref().is_some_and(|p| {
                log2_cb >= u32::from(p.log2_min_size) && log2_cb <= u32::from(p.log2_max_size)
            }) && self.cabac.terminate();
        if pcm {
            self.pcm_samples(x0, y0, log2_cb)?;
        } else {
            self.intra_modes(x0, y0, size, nxn);
            let max_depth = u32::from(sps.max_transform_hierarchy_depth_intra) + u32::from(nxn);
            self.transform_tree(x0, y0, x0, y0, log2_cb, 0, 0, max_depth, 1, 1)?;
        }
        Picture::fill(&mut self.pic.qp_y, w4, x0, y0, size, size, self.qp_y as i8);
        self.last_qp_y = self.qp_y;
        Ok(())
    }

    fn pcm_samples(&mut self, x0: usize, y0: usize, log2_cb: u32) -> Result<(), Error> {
        let pcm = self
            .sps
            .pcm
            .clone()
            .ok_or_else(|| bad("PCM without parameters"))?;
        let size = 1usize << log2_cb;
        let start = self.cabac.byte_position();
        let mut bits = Bits::new(self.cabac.remaining());
        let components = if self.sps.chroma_array_type != 0 {
            3
        } else {
            1
        };
        for c in 0..components {
            let (sw, sh) = if c == 0 {
                (1, 1)
            } else {
                (
                    self.sps.sub_width() as usize,
                    self.sps.sub_height() as usize,
                )
            };
            let (stored, depth) = if c == 0 {
                (u32::from(pcm.bit_depth_luma), self.sps.bit_depth_luma)
            } else {
                (u32::from(pcm.bit_depth_chroma), self.sps.bit_depth_chroma)
            };
            let shift = u32::from(depth) - stored;
            let stride = self.pic.plane_width[c];
            for y in 0..size / sh {
                for x in 0..size / sw {
                    let value = bits.u(stored)? << shift;
                    self.pic.planes[c][(y0 / sh + y) * stride + x0 / sw + x] = value as u16;
                }
            }
        }
        self.cabac.set_byte_position(start + bits.byte_position());
        self.cabac.start();
        let w4 = self.pic.w4;
        Picture::fill(&mut self.pic.intra_mode, w4, x0, y0, size, size, 1);
        if pcm.loop_filter_disabled {
            Picture::fill(
                &mut self.pic.flags,
                w4,
                x0,
                y0,
                size,
                size,
                FLAG_PCM_NO_FILTER,
            );
        }
        self.mark_transform_block(x0, y0, size);
        Ok(())
    }

    /// Neighbouring intra mode for the most probable mode list: the mode of an
    /// already decoded block of this slice, or of an earlier part of this CU.
    fn neighbour_mode(&self, x: i64, y: i64) -> u8 {
        let (cx, cy, log2) = self.cu;
        let size = 1i64 << log2;
        let inside =
            x >= cx as i64 && x < cx as i64 + size && y >= cy as i64 && y < cy as i64 + size;
        if (inside && x >= 0 && y >= 0) || self.pic.available(self.slice_address, x, y) {
            self.pic.intra_mode[self.pic.at4(x as usize, y as usize)]
        } else {
            1
        }
    }

    fn intra_modes(&mut self, x0: usize, y0: usize, size: usize, nxn: bool) {
        let parts = if nxn { 4 } else { 1 };
        let part_size = if nxn { size / 2 } else { size };
        let mut flags = [false; 4];
        for flag in flags.iter_mut().take(parts) {
            *flag = self.cabac.bin(&mut self.models.0[ctx::PREV_INTRA_LUMA]) == 1;
        }
        let w4 = self.pic.w4;
        let ctb_top = (y0 >> self.sps.log2_ctb) << self.sps.log2_ctb;
        let mut luma = [1u8; 4];
        for (i, &flag) in flags.iter().enumerate().take(parts) {
            let (px, py) = (x0 + (i & 1) * part_size, y0 + (i >> 1) * part_size);
            let a = self.neighbour_mode(px as i64 - 1, py as i64);
            // The row above may not belong to another CTB row.
            let b = if py > ctb_top {
                self.neighbour_mode(px as i64, py as i64 - 1)
            } else {
                1
            };
            let mut candidates = if a == b {
                if a < 2 {
                    [0, 1, 26]
                } else {
                    [a, 2 + ((a + 29) % 32), 2 + ((a - 2 + 1) % 32)]
                }
            } else {
                let third = if a != 0 && b != 0 {
                    0
                } else if a != 1 && b != 1 {
                    1
                } else {
                    26
                };
                [a, b, third]
            };
            let mode = if flag {
                // mpm_idx: truncated unary with at most two bypass bins.
                let index = if self.cabac.bypass() == 0 {
                    0
                } else {
                    1 + self.cabac.bypass() as usize
                };
                candidates[index]
            } else {
                let mut mode = self.cabac.bypass_bits(5) as u8;
                candidates.sort_unstable();
                for candidate in candidates {
                    if mode >= candidate {
                        mode += 1;
                    }
                }
                mode
            };
            luma[i] = mode;
            Picture::fill(
                &mut self.pic.intra_mode,
                w4,
                px,
                py,
                part_size,
                part_size,
                mode,
            );
        }
        if self.sps.chroma_array_type == 0 {
            return;
        }
        let count = if self.sps.chroma_array_type == 3 && nxn {
            4
        } else {
            1
        };
        for i in 0..count {
            let index = if self.cabac.bin(&mut self.models.0[ctx::INTRA_CHROMA]) == 0 {
                4
            } else {
                self.cabac.bypass_bits(2) as usize
            };
            let from_luma = luma[if count == 4 { i } else { 0 }];
            let mut mode = match index {
                4 => from_luma,
                _ => {
                    let chosen = [0u8, 26, 10, 1][index];
                    if chosen == from_luma { 34 } else { chosen }
                }
            };
            if self.sps.chroma_array_type == 2 {
                mode = MODE_422[mode as usize];
            }
            self.chroma_modes[i] = mode;
        }
        if count == 1 {
            self.chroma_modes = [self.chroma_modes[0]; 4];
        }
    }

    // --- transform tree and unit (7.3.8.8, 7.3.8.10) ---

    #[allow(clippy::too_many_arguments)]
    fn transform_tree(
        &mut self,
        x0: usize,
        y0: usize,
        x_base: usize,
        y_base: usize,
        log2: u32,
        depth: u32,
        blk_idx: usize,
        max_depth: u32,
        parent_cb: u8,
        parent_cr: u8,
    ) -> Result<(), Error> {
        let sps = self.sps;
        let chroma = sps.chroma_array_type;
        let split = if log2 <= u32::from(sps.log2_max_tb)
            && log2 > u32::from(sps.log2_min_tb)
            && depth < max_depth
            && !(self.intra_split && depth == 0)
        {
            self.cabac
                .bin(&mut self.models.0[ctx::SPLIT_TRANSFORM + (5 - log2) as usize])
                == 1
        } else {
            log2 > u32::from(sps.log2_max_tb) || (self.intra_split && depth == 0)
        };
        let (mut cb, mut cr) = (0u8, 0u8);
        if (log2 > 2 && chroma != 0) || chroma == 3 {
            let second = chroma == 2 && (!split || log2 == 3);
            for (flag, parent) in [(&mut cb, parent_cb), (&mut cr, parent_cr)] {
                if parent != 0 {
                    *flag = self
                        .cabac
                        .bin(&mut self.models.0[ctx::CBF_CHROMA + depth as usize])
                        as u8;
                    if second {
                        *flag |= (self
                            .cabac
                            .bin(&mut self.models.0[ctx::CBF_CHROMA + depth as usize])
                            as u8)
                            << 1;
                    }
                }
            }
        } else if depth > 0 && log2 == 2 {
            // Chroma of 4x4 luma blocks is coded with their parent.
            cb = parent_cb;
            cr = parent_cr;
        }
        if split {
            let half = 1usize << (log2 - 1);
            for (i, (dx, dy)) in [(0, 0), (half, 0), (0, half), (half, half)]
                .into_iter()
                .enumerate()
            {
                self.transform_tree(
                    x0 + dx,
                    y0 + dy,
                    x0,
                    y0,
                    log2 - 1,
                    depth + 1,
                    i,
                    max_depth,
                    cb,
                    cr,
                )?;
            }
            Ok(())
        } else {
            let cbf_luma = self
                .cabac
                .bin(&mut self.models.0[ctx::CBF_LUMA + usize::from(depth == 0)])
                == 1;
            self.transform_unit(x0, y0, x_base, y_base, log2, blk_idx, cbf_luma, cb, cr)
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn transform_unit(
        &mut self,
        x0: usize,
        y0: usize,
        x_base: usize,
        y_base: usize,
        log2: u32,
        blk_idx: usize,
        cbf_luma: bool,
        cb: u8,
        cr: u8,
    ) -> Result<(), Error> {
        let sps = self.sps;
        let chroma = sps.chroma_array_type;
        let cbf_chroma = cb != 0 || cr != 0;
        if cbf_luma || cbf_chroma {
            if self.pps.cu_qp_delta && !self.cu_qp_delta_coded {
                let delta = self.cu_qp_delta_value()?;
                self.cu_qp_delta_coded = true;
                let offset = sps.qp_bd_offset_luma();
                self.qp_y = ((self.qp_y_pred + delta + 52 + 2 * offset) % (52 + offset)) - offset;
            }
            if self.header.cu_chroma_qp_offset_enabled
                && cbf_chroma
                && !self.bypass
                && !self.cu_chroma_qp_offset_coded
            {
                let flag = self
                    .cabac
                    .bin(&mut self.models.0[ctx::CU_CHROMA_QP_OFFSET_FLAG])
                    == 1;
                let mut index = 0;
                if flag && self.pps.cb_qp_offset_list.len() > 1 {
                    index = self
                        .cabac
                        .bin(&mut self.models.0[ctx::CU_CHROMA_QP_OFFSET_IDX])
                        as usize;
                }
                self.cu_chroma_qp_offset_coded = true;
                self.cu_qp_offset = if flag {
                    [
                        self.pps.cb_qp_offset_list[index],
                        self.pps.cr_qp_offset_list[index],
                    ]
                } else {
                    [0, 0]
                };
            }
        }
        let n = 1usize << log2;
        let (cu_x, cu_y, cu_log2) = self.cu;
        // Luma.
        let mode = self.pic.intra_mode[self.pic.at4(x0, y0)];
        let disable_boundary = sps.range.implicit_rdpcm && self.bypass;
        self.pic
            .predict_intra(self.slice_address, 0, x0, y0, n, mode, disable_boundary);
        if cbf_luma {
            let block = self.residual_coding(0, log2, mode)?;
            self.reconstruct(0, x0, y0, log2, &block, mode);
        }
        self.mark_transform_block(x0, y0, n);
        if chroma == 0 {
            return Ok(());
        }
        // Chroma: with the block, or for four 4x4 luma blocks after the last one.
        let (cx0, cy0, log2c) = if log2 > 2 || chroma == 3 {
            (x0, y0, if chroma == 3 { log2 } else { log2 - 1 })
        } else if blk_idx == 3 {
            (x_base, y_base, 2)
        } else {
            return Ok(());
        };
        let part = if chroma == 3 && self.intra_split {
            let half = 1usize << (cu_log2 - 1);
            usize::from(x0 >= cu_x + half) + 2 * usize::from(y0 >= cu_y + half)
        } else {
            0
        };
        let chroma_mode = self.chroma_modes[part];
        let (sw, sh) = (sps.sub_width() as usize, sps.sub_height() as usize);
        let nc = 1usize << log2c;
        for (c, flags) in [(1usize, cb), (2usize, cr)] {
            for t in 0..if chroma == 2 { 2 } else { 1 } {
                let (bx, by) = (cx0 / sw, cy0 / sh + t * nc);
                self.pic.predict_intra(
                    self.slice_address,
                    c,
                    bx,
                    by,
                    nc,
                    chroma_mode,
                    disable_boundary,
                );
                if flags >> t & 1 == 1 {
                    let block = self.residual_coding(c, log2c, chroma_mode)?;
                    self.reconstruct(c, bx, by, log2c, &block, chroma_mode);
                }
            }
        }
        Ok(())
    }

    /// Records a reconstructed luma transform block: it is now available for
    /// prediction, and its left and top sides are transform block edges.
    fn mark_transform_block(&mut self, x0: usize, y0: usize, n: usize) {
        let w4 = self.pic.w4;
        Picture::fill(&mut self.pic.decoded, w4, x0, y0, n, n, true);
        for offset in (0..n).step_by(4) {
            let left = self.pic.at4(x0, y0 + offset);
            let top = self.pic.at4(x0 + offset, y0);
            if y0 + offset < self.sps.height as usize {
                self.pic.edges[left] |= EDGE_VERTICAL;
            }
            if x0 + offset < self.sps.width as usize {
                self.pic.edges[top] |= EDGE_HORIZONTAL;
            }
        }
    }

    fn cu_qp_delta_value(&mut self) -> Result<i32, Error> {
        // Prefix: truncated unary with five bins, the first with its own
        // context and the rest sharing another. Suffix: Exp-Golomb of order 0.
        let mut value = 0u32;
        let mut bin = self.cabac.bin(&mut self.models.0[ctx::CU_QP_DELTA]);
        while bin == 1 && value < 5 {
            value += 1;
            if value < 5 {
                bin = self.cabac.bin(&mut self.models.0[ctx::CU_QP_DELTA + 1]);
            }
        }
        if value == 5 {
            let mut k = 0;
            while self.cabac.bypass() == 1 {
                value += 1 << k;
                k += 1;
                if k > 30 {
                    return Err(bad("cu_qp_delta_abs is too large"));
                }
            }
            if k > 0 {
                value += self.cabac.bypass_bits(k);
            }
        }
        let offset = self.sps.qp_bd_offset_luma();
        let negative = value > 0 && self.cabac.bypass() == 1;
        let limit = if negative { 26 } else { 25 } + offset / 2;
        if value as i32 > limit {
            return Err(bad("cu_qp_delta_abs is out of range"));
        }
        Ok(if negative {
            -(value as i32)
        } else {
            value as i32
        })
    }

    // --- residual coding (7.3.8.11) ---

    fn residual_coding(&mut self, c: usize, log2: u32, mode: u8) -> Result<Block, Error> {
        let sps = self.sps;
        let n = 1usize << log2;
        self.coefficients[..n * n].fill(0);
        let mut transform_skip = false;
        if self.pps.transform_skip && !self.bypass && log2 <= self.pps.log2_max_transform_skip_size
        {
            transform_skip = self
                .cabac
                .bin(&mut self.models.0[ctx::TRANSFORM_SKIP + usize::from(c > 0)])
                == 1;
        }
        // Where the last significant coefficient is.
        let (offset, shift) = if c == 0 {
            (3 * (log2 - 2) + ((log2 - 1) >> 2), (log2 + 1) >> 2)
        } else {
            (15, log2 - 2)
        };
        let max_prefix = (log2 << 1) - 1;
        let mut last = [0u32; 2];
        for (axis, base) in [ctx::LAST_X_PREFIX, ctx::LAST_Y_PREFIX]
            .into_iter()
            .enumerate()
        {
            while last[axis] < max_prefix
                && self
                    .cabac
                    .bin(&mut self.models.0[base + (offset + (last[axis] >> shift)) as usize])
                    == 1
            {
                last[axis] += 1;
            }
        }
        for position in &mut last {
            if *position > 3 {
                let bits = (*position >> 1) - 1;
                *position = ((2 + (*position & 1)) << bits) + self.cabac.bypass_bits(bits);
            }
        }
        let scan_index = if (log2 == 2 || (log2 == 3 && (c == 0 || sps.chroma_array_type == 3)))
            && (6..=14).contains(&mode)
        {
            2
        } else if (log2 == 2 || (log2 == 3 && (c == 0 || sps.chroma_array_type == 3)))
            && (22..=30).contains(&mode)
        {
            1
        } else {
            0
        };
        let (mut last_x, mut last_y) = (last[0] as usize, last[1] as usize);
        if scan_index == 2 {
            std::mem::swap(&mut last_x, &mut last_y);
        }
        if last_x >= n || last_y >= n {
            return Err(bad("the last coefficient lies outside its block"));
        }
        let sub_scan = &scans()[(log2 - 2) as usize][scan_index];
        let pos_scan = &scans()[2][scan_index];
        let last_sub = sub_scan
            .iter()
            .position(|&(x, y)| usize::from(x) == last_x >> 2 && usize::from(y) == last_y >> 2)
            .ok_or_else(|| bad("no sub-block holds the last coefficient"))?;
        let last_pos = pos_scan
            .iter()
            .position(|&(x, y)| usize::from(x) == last_x & 3 && usize::from(y) == last_y & 3)
            .ok_or_else(|| bad("no position holds the last coefficient"))?;

        let sb_width = 1usize << (log2 - 2);
        let mut coded = [false; 64];
        let skip_context = sps.range.transform_skip_context && (self.bypass || transform_skip);
        let intra_rdpcm_blocks_hiding = self.bypass
            || (sps.range.implicit_rdpcm && transform_skip && (mode == 10 || mode == 26));
        let sb_type = usize::from(c == 0) * 2 + usize::from(transform_skip || self.bypass);
        let mut c1 = 1u32;
        let (mut max_x, mut max_y) = (0usize, 0usize);

        for i in (0..=last_sub).rev() {
            let (xs, ys) = (usize::from(sub_scan[i].0), usize::from(sub_scan[i].1));
            let right = xs + 1 < sb_width && coded[ys * sb_width + xs + 1];
            let below = ys + 1 < sb_width && coded[(ys + 1) * sb_width + xs];
            let mut infer_dc = false;
            let sub_coded = if i < last_sub && i > 0 {
                let increment = usize::from(right || below) + if c > 0 { 2 } else { 0 };
                infer_dc = true;
                self.cabac
                    .bin(&mut self.models.0[ctx::CODED_SUB_BLOCK + increment])
                    == 1
            } else {
                true
            };
            coded[ys * sb_width + xs] = sub_coded;
            if !sub_coded {
                continue;
            }
            let previous = usize::from(right) | usize::from(below) << 1;
            // Significant coefficients of this sub-block, from the highest scan
            // position down: (scan position, level).
            let mut positions = [0u8; 16];
            let mut count = 0;
            let sig = |this: &mut Self, p: usize| -> bool {
                let (xp, yp) = (usize::from(pos_scan[p].0), usize::from(pos_scan[p].1));
                let increment = if skip_context {
                    if c == 0 { 42 } else { 43 }
                } else {
                    let base = if log2 == 2 {
                        usize::from(SIG_CTX_4X4[(yp << 2) + xp])
                    } else if xs + ys + xp + yp == 0 {
                        0
                    } else {
                        let mut s = match previous {
                            0 => {
                                if xp + yp == 0 {
                                    2
                                } else if xp + yp < 3 {
                                    1
                                } else {
                                    0
                                }
                            }
                            1 => 2usize.saturating_sub(yp),
                            2 => 2usize.saturating_sub(xp),
                            _ => 2,
                        };
                        if c == 0 {
                            if xs + ys > 0 {
                                s += 3;
                            }
                            s += if log2 == 3 {
                                if scan_index == 0 { 9 } else { 15 }
                            } else {
                                21
                            };
                        } else {
                            s += if log2 == 3 { 9 } else { 12 };
                        }
                        s
                    };
                    if c == 0 { base } else { 27 + base }
                };
                this.cabac
                    .bin(&mut this.models.0[ctx::SIG_COEFF + increment])
                    == 1
            };
            let first = if i == last_sub {
                positions[0] = last_pos as u8;
                count = 1;
                last_pos as i32 - 1
            } else {
                15
            };
            for p in (1..=first).rev() {
                if sig(self, p as usize) {
                    positions[count] = p as u8;
                    count += 1;
                    infer_dc = false;
                }
            }
            if first >= 0 {
                // The DC position of the sub-block.
                if !infer_dc {
                    if sig(self, 0) {
                        positions[count] = 0;
                        count += 1;
                    }
                } else {
                    positions[count] = 0;
                    count += 1;
                }
            }
            if count == 0 {
                continue;
            }
            // Greater-than-one flags of the first eight, one greater-than-two flag.
            let mut set = if i == 0 || c > 0 { 0 } else { 2 };
            if c1 == 0 {
                set += 1;
            }
            c1 = 1;
            let mut levels = [1i32; 16];
            let mut has_remaining = [false; 16];
            let mut first_greater1 = None;
            for k in 0..count.min(8) {
                let increment = set * 4 + c1.min(3) as usize + if c > 0 { 16 } else { 0 };
                if self
                    .cabac
                    .bin(&mut self.models.0[ctx::GREATER1 + increment])
                    == 1
                {
                    levels[k] = 2;
                    c1 = 0;
                    if first_greater1.is_none() {
                        first_greater1 = Some(k);
                    }
                    has_remaining[k] = true;
                } else if c1 > 0 && c1 < 3 {
                    c1 += 1;
                }
            }
            for flag in has_remaining.iter_mut().take(count).skip(8) {
                *flag = true;
            }
            if let Some(k) = first_greater1 {
                let increment = set + if c > 0 { 4 } else { 0 };
                if self
                    .cabac
                    .bin(&mut self.models.0[ctx::GREATER2 + increment])
                    == 1
                {
                    levels[k] = 3;
                } else {
                    has_remaining[k] = false;
                }
                // Further coefficients with greater-than-one keep their escape.
            }
            // Signs.
            let hidden = self.pps.sign_data_hiding
                && !intra_rdpcm_blocks_hiding
                && positions[0] as i32 - positions[count - 1] as i32 > 3;
            let sign_count = count - usize::from(hidden);
            let signs =
                self.cabac.bypass_bits(sign_count as u32) << (32 - sign_count as u32).min(31);
            let sign_at = |k: usize| -> bool {
                k < sign_count && (signs >> (31 - k as u32)) & 1 == 1 && sign_count > 0
            };
            // Remaining levels.
            let mut rice = if sps.range.persistent_rice_adaptation {
                u32::from(self.stat_coeff[sb_type] / 4)
            } else {
                0
            };
            let mut first_remaining = true;
            let mut sum = 0i32;
            for k in 0..count {
                let base = levels[k];
                let mut level = base;
                if has_remaining[k] {
                    let remaining = self.coeff_abs_level_remaining(rice)? as i32;
                    level = base + remaining;
                    if sps.range.persistent_rice_adaptation {
                        if first_remaining {
                            let stat = &mut self.stat_coeff[sb_type];
                            if remaining >= (3 << (*stat / 4)) {
                                *stat = (*stat + 1).min(119);
                            } else if 2 * remaining < (1 << (*stat / 4)) && *stat > 0 {
                                *stat -= 1;
                            }
                        }
                        if level > 3 * (1 << rice) {
                            rice = (rice + 1).min(29);
                        }
                    } else if level > 3 * (1 << rice) {
                        rice = (rice + 1).min(4);
                    }
                    first_remaining = false;
                }
                let mut value = if sign_at(k) { -level } else { level };
                if hidden {
                    sum += value;
                    if k == count - 1 && sum & 1 == 1 {
                        value = -value;
                    }
                }
                let p = positions[k] as usize;
                let x = (xs << 2) + usize::from(pos_scan[p].0);
                let y = (ys << 2) + usize::from(pos_scan[p].1);
                self.coefficients[y * n + x] = value.clamp(-32768, 32767);
                max_x = max_x.max(x);
                max_y = max_y.max(y);
            }
        }
        Ok(Block {
            transform_skip,
            max_x,
            max_y,
        })
    }

    fn coeff_abs_level_remaining(&mut self, rice: u32) -> Result<u32, Error> {
        let mut prefix = 0u32;
        while self.cabac.bypass() == 1 {
            prefix += 1;
            if prefix > 32 {
                return Err(bad("a coefficient level is too large"));
            }
        }
        if prefix <= 3 {
            Ok((prefix << rice) + self.cabac.bypass_bits(rice))
        } else {
            let bits = prefix - 3 + rice;
            if bits > 31 {
                return Err(bad("a coefficient level is too large"));
            }
            Ok((((1u32 << (prefix - 3)) + 3 - 1) << rice) + self.cabac.bypass_bits(bits))
        }
    }

    // --- scaling, transform and reconstruction (8.6) ---

    fn reconstruct(&mut self, c: usize, x: usize, y: usize, log2: u32, block: &Block, mode: u8) {
        let n = 1usize << log2;
        let sps = self.sps;
        let depth = if c == 0 {
            sps.bit_depth_luma
        } else {
            sps.bit_depth_chroma
        };
        let rotate = sps.range.transform_skip_rotation && n == 4;
        let rdpcm = if sps.range.implicit_rdpcm
            && (self.bypass || block.transform_skip)
            && (mode == 10 || mode == 26)
        {
            Some(mode == 26)
        } else {
            None
        };
        let count = n * n;
        if self.bypass {
            if rotate {
                self.coefficients[..count].reverse();
            }
            self.residual[..count].copy_from_slice(&self.coefficients[..count]);
        } else {
            // Scaling (8.6.3).
            let qp = if c == 0 {
                self.qp_y + sps.qp_bd_offset_luma()
            } else {
                self.chroma_qp(c)
            };
            let shift = u32::from(depth) + log2 - 5;
            let round = 1i64 << (shift - 1);
            let flat = self.scaling.is_none() || (block.transform_skip && n > 4);
            let factor = i64::from(LEVEL_SCALE[(qp % 6) as usize]) << (qp / 6);
            let lists = self.scaling.map(|l| &l.factors[(log2 - 2) as usize][c]);
            for yy in 0..=block.max_y {
                for xx in 0..=block.max_x {
                    let level = self.coefficients[yy * n + xx];
                    if level != 0 {
                        let m = if flat {
                            16
                        } else {
                            i64::from(lists.map_or(16, |l| l[yy * n + xx]))
                        };
                        let scaled = (i64::from(level) * m * factor + round) >> shift;
                        self.coefficients[yy * n + xx] = scaled.clamp(-32768, 32767) as i32;
                    }
                }
            }
            if block.transform_skip {
                if rotate {
                    self.coefficients[..count].reverse();
                }
                let bd_shift = 20 - u32::from(depth);
                let ts_shift = 5 + log2;
                let round = 1i64 << (bd_shift - 1);
                for i in 0..count {
                    self.residual[i] = (((i64::from(self.coefficients[i]) << ts_shift) + round)
                        >> bd_shift) as i32;
                }
            } else {
                let dst = n == 4 && c == 0;
                transform::inverse(
                    &self.coefficients,
                    n,
                    block.max_x,
                    block.max_y,
                    dst,
                    depth,
                    &mut self.residual,
                );
            }
        }
        if let Some(vertical) = rdpcm {
            for yy in 0..n {
                for xx in 0..n {
                    if vertical && yy > 0 {
                        self.residual[yy * n + xx] += self.residual[(yy - 1) * n + xx];
                    } else if !vertical && xx > 0 {
                        self.residual[yy * n + xx] += self.residual[yy * n + xx - 1];
                    }
                }
            }
        }
        // Add to the prediction.
        let max = (1i32 << depth) - 1;
        let stride = self.pic.plane_width[c];
        let plane = &mut self.pic.planes[c];
        for yy in 0..n {
            for xx in 0..n {
                let at = (y + yy) * stride + x + xx;
                plane[at] =
                    (i32::from(plane[at]) + self.residual[yy * n + xx]).clamp(0, max) as u16;
            }
        }
    }
}
