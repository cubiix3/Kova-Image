//! The in-loop filters, applied to the whole picture after all slices are
//! decoded: deblocking (8.7.2) and sample adaptive offset (8.7.3).
use super::picture::{EDGE_HORIZONTAL, EDGE_VERTICAL, FLAG_BYPASS, FLAG_PCM_NO_FILTER, Picture};

const BETA: [u8; 52] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18,
    20, 22, 24, 26, 28, 30, 32, 34, 36, 38, 40, 42, 44, 46, 48, 50, 52, 54, 56, 58, 60, 62, 64,
];
const TC: [u8; 54] = [
    0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 2, 2, 2, 2, 3,
    3, 3, 3, 4, 4, 4, 5, 5, 6, 6, 7, 8, 9, 10, 11, 13, 14, 16, 18, 20, 22, 24,
];
const CHROMA_QP: [i32; 14] = [29, 30, 31, 32, 33, 33, 34, 34, 35, 35, 36, 36, 37, 37];

impl Picture<'_> {
    /// True if the filters may change the samples of the 4x4 block at (`x`, `y`).
    fn filterable(&self, x: usize, y: usize) -> bool {
        self.flags[self.at4(x, y)] & (FLAG_BYPASS | FLAG_PCM_NO_FILTER) == 0
    }

    /// Whether the edge of the block at (`x`, `y`) towards the left (or top) may
    /// be filtered at all: not for a slice that has deblocking off, and not
    /// across a slice boundary that the slice forbids.
    fn edge_allowed(&self, x: usize, y: usize, vertical: bool) -> bool {
        let q = self.ctb_slice[self.ctb_of(x, y)];
        let Some(slice) = usize::try_from(q).ok().map(|i| &self.slices[i]) else {
            return false;
        };
        if slice.deblocking_disabled {
            return false;
        }
        let mask = (1usize << self.sps.log2_ctb) - 1;
        let on_ctb_boundary = if vertical {
            x & mask == 0
        } else {
            y & mask == 0
        };
        if on_ctb_boundary && !slice.loop_filter_across_slices {
            let (px, py) = if vertical { (x - 1, y) } else { (x, y - 1) };
            let p = self.ctb_slice[self.ctb_of(px, py)];
            if let Ok(p) = usize::try_from(p)
                && self.slices[p].address != slice.address
            {
                return false;
            }
        }
        true
    }

    pub fn deblock(&mut self) {
        if self.slices.iter().all(|s| s.deblocking_disabled) {
            return;
        }
        for vertical in [true, false] {
            self.deblock_luma(vertical);
            if self.sps.chroma_array_type != 0 {
                self.deblock_chroma(vertical);
            }
        }
    }

    fn deblock_luma(&mut self, vertical: bool) {
        let (w, h) = (self.sps.width as usize, self.sps.height as usize);
        let depth = self.sps.bit_depth_luma;
        let stride = self.plane_width[0];
        let flag = if vertical {
            EDGE_VERTICAL
        } else {
            EDGE_HORIZONTAL
        };
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                // Edges lie on an 8x8 grid; the picture border is not an edge.
                let across = if vertical { x } else { y };
                if across == 0 || across % 8 != 0 {
                    continue;
                }
                if self.edges[self.at4(x, y)] & flag == 0 || !self.edge_allowed(x, y, vertical) {
                    continue;
                }
                let (px, py) = if vertical { (x - 1, y) } else { (x, y - 1) };
                let qp = (i32::from(self.qp_y[self.at4(x, y)])
                    + i32::from(self.qp_y[self.at4(px, py)])
                    + 1)
                    >> 1;
                let slice = &self.slices[self.ctb_slice[self.ctb_of(x, y)] as usize];
                let beta = i32::from(BETA[(qp + slice.beta_offset_div2 * 2).clamp(0, 51) as usize])
                    << (depth - 8);
                // bS is 2 on every edge of an intra picture.
                let tc = i32::from(TC[(qp + 2 + slice.tc_offset_div2 * 2).clamp(0, 53) as usize])
                    << (depth - 8);
                let filter_p = self.filterable(px, py);
                let filter_q = self.filterable(x, y);
                let max = (1i32 << depth) - 1;
                let base = y * stride + x;
                let plane = &mut self.planes[0];
                // Sample `i` steps from the edge on line `k`; `i` below 0 is on the P side.
                let at = |k: usize, i: i32| -> usize {
                    if vertical {
                        (base as isize + (k * stride) as isize + i as isize) as usize
                    } else {
                        (base as isize + k as isize + i as isize * stride as isize) as usize
                    }
                };
                let sample = |plane: &Vec<u16>, k: usize, i: i32| i32::from(plane[at(k, i)]);
                let p = |plane: &Vec<u16>, k: usize, i: i32| sample(plane, k, -1 - i);
                let q = |plane: &Vec<u16>, k: usize, i: i32| sample(plane, k, i);
                let dp0 = (p(plane, 0, 2) - 2 * p(plane, 0, 1) + p(plane, 0, 0)).abs();
                let dp3 = (p(plane, 3, 2) - 2 * p(plane, 3, 1) + p(plane, 3, 0)).abs();
                let dq0 = (q(plane, 0, 2) - 2 * q(plane, 0, 1) + q(plane, 0, 0)).abs();
                let dq3 = (q(plane, 3, 2) - 2 * q(plane, 3, 1) + q(plane, 3, 0)).abs();
                let (dpq0, dpq3) = (dp0 + dq0, dp3 + dq3);
                if dpq0 + dpq3 >= beta {
                    continue;
                }
                let line_is_smooth = |plane: &Vec<u16>, k: usize, dpq: i32| {
                    2 * dpq < (beta >> 2)
                        && (p(plane, k, 3) - p(plane, k, 0)).abs()
                            + (q(plane, k, 0) - q(plane, k, 3)).abs()
                            < (beta >> 3)
                        && (p(plane, k, 0) - q(plane, k, 0)).abs() < ((5 * tc + 1) >> 1)
                };
                let strong = line_is_smooth(plane, 0, dpq0) && line_is_smooth(plane, 3, dpq3);
                let side_limit = (beta + (beta >> 1)) >> 3;
                let (filter_p1, filter_q1) = (dp0 + dp3 < side_limit, dq0 + dq3 < side_limit);
                for k in 0..4 {
                    let (p0, p1, p2, p3) = (
                        p(plane, k, 0),
                        p(plane, k, 1),
                        p(plane, k, 2),
                        p(plane, k, 3),
                    );
                    let (q0, q1, q2, q3) = (
                        q(plane, k, 0),
                        q(plane, k, 1),
                        q(plane, k, 2),
                        q(plane, k, 3),
                    );
                    if strong {
                        let new_p = [
                            ((p2 + 2 * p1 + 2 * p0 + 2 * q0 + q1 + 4) >> 3)
                                .clamp(p0 - 2 * tc, p0 + 2 * tc),
                            ((p2 + p1 + p0 + q0 + 2) >> 2).clamp(p1 - 2 * tc, p1 + 2 * tc),
                            ((2 * p3 + 3 * p2 + p1 + p0 + q0 + 4) >> 3)
                                .clamp(p2 - 2 * tc, p2 + 2 * tc),
                        ];
                        let new_q = [
                            ((p1 + 2 * p0 + 2 * q0 + 2 * q1 + q2 + 4) >> 3)
                                .clamp(q0 - 2 * tc, q0 + 2 * tc),
                            ((p0 + q0 + q1 + q2 + 2) >> 2).clamp(q1 - 2 * tc, q1 + 2 * tc),
                            ((p0 + q0 + q1 + 3 * q2 + 2 * q3 + 4) >> 3)
                                .clamp(q2 - 2 * tc, q2 + 2 * tc),
                        ];
                        for i in 0..3 {
                            if filter_p {
                                plane[at(k, -1 - i as i32)] = new_p[i] as u16;
                            }
                            if filter_q {
                                plane[at(k, i as i32)] = new_q[i] as u16;
                            }
                        }
                    } else {
                        let mut delta = (9 * (q0 - p0) - 3 * (q1 - p1) + 8) >> 4;
                        if delta.abs() >= tc * 10 {
                            continue;
                        }
                        delta = delta.clamp(-tc, tc);
                        if filter_p {
                            plane[at(k, -1)] = (p0 + delta).clamp(0, max) as u16;
                            if filter_p1 {
                                let d = ((((p2 + p0 + 1) >> 1) - p1 + delta) >> 1)
                                    .clamp(-(tc >> 1), tc >> 1);
                                plane[at(k, -2)] = (p1 + d).clamp(0, max) as u16;
                            }
                        }
                        if filter_q {
                            plane[at(k, 0)] = (q0 - delta).clamp(0, max) as u16;
                            if filter_q1 {
                                let d = ((((q2 + q0 + 1) >> 1) - q1 - delta) >> 1)
                                    .clamp(-(tc >> 1), tc >> 1);
                                plane[at(k, 1)] = (q1 + d).clamp(0, max) as u16;
                            }
                        }
                    }
                }
            }
        }
    }

    fn deblock_chroma(&mut self, vertical: bool) {
        let (w, h) = (self.sps.width as usize, self.sps.height as usize);
        let (sw, sh) = (
            self.sps.sub_width() as usize,
            self.sps.sub_height() as usize,
        );
        let depth = self.sps.bit_depth_chroma;
        let flag = if vertical {
            EDGE_VERTICAL
        } else {
            EDGE_HORIZONTAL
        };
        let max = (1i32 << depth) - 1;
        for y in (0..h).step_by(4) {
            for x in (0..w).step_by(4) {
                // The chroma edge grid is 8 chroma samples apart.
                let across = if vertical { x / sw } else { y / sh };
                if across == 0 || across % 8 != 0 {
                    continue;
                }
                if self.edges[self.at4(x, y)] & flag == 0 || !self.edge_allowed(x, y, vertical) {
                    continue;
                }
                let (px, py) = if vertical { (x - 1, y) } else { (x, y - 1) };
                let filter_p = self.filterable(px, py);
                let filter_q = self.filterable(x, y);
                let qp_sum = (i32::from(self.qp_y[self.at4(x, y)])
                    + i32::from(self.qp_y[self.at4(px, py)])
                    + 1)
                    >> 1;
                let slice = &self.slices[self.ctb_slice[self.ctb_of(x, y)] as usize];
                for c in 1..3 {
                    let offset = if c == 1 {
                        self.pps.cb_qp_offset
                    } else {
                        self.pps.cr_qp_offset
                    };
                    let qpi = qp_sum + offset;
                    let qpc = if self.sps.chroma_array_type == 1 {
                        match qpi {
                            ..30 => qpi,
                            30..=43 => CHROMA_QP[(qpi - 30) as usize],
                            _ => qpi - 6,
                        }
                    } else {
                        qpi.min(51)
                    };
                    let tc =
                        i32::from(TC[(qpc + 2 + slice.tc_offset_div2 * 2).clamp(0, 53) as usize])
                            << (depth - 8);
                    let stride = self.plane_width[c];
                    let (cx, cy) = (x / sw, y / sh);
                    // Lines of this 4x4 luma block along the edge.
                    let lines = if vertical { 4 / sh } else { 4 / sw };
                    let plane = &mut self.planes[c];
                    for k in 0..lines {
                        let (step, line) = if vertical {
                            (1isize, (cy + k) * stride + cx)
                        } else {
                            (stride as isize, cy * stride + cx + k)
                        };
                        let at = |i: isize| (line as isize + i * step) as usize;
                        let (p0, p1) = (i32::from(plane[at(-1)]), i32::from(plane[at(-2)]));
                        let (q0, q1) = (i32::from(plane[at(0)]), i32::from(plane[at(1)]));
                        let delta = ((((q0 - p0) << 2) + p1 - q1 + 4) >> 3).clamp(-tc, tc);
                        if filter_p {
                            plane[at(-1)] = (p0 + delta).clamp(0, max) as u16;
                        }
                        if filter_q {
                            plane[at(0)] = (q0 - delta).clamp(0, max) as u16;
                        }
                    }
                }
            }
        }
    }

    /// Sample adaptive offset (8.7.3), reading the deblocked samples and writing
    /// the result.
    pub fn apply_sao(&mut self) {
        if self.slices.is_empty() {
            return;
        }
        let components = if self.sps.chroma_array_type != 0 {
            3
        } else {
            1
        };
        let ctb = 1usize << self.sps.log2_ctb;
        for c in 0..components {
            if self.sao.iter().all(|s| s[c].kind == 0) {
                continue;
            }
            let source = self.planes[c].clone();
            let (sw, sh) = if c == 0 {
                (1, 1)
            } else {
                (
                    self.sps.sub_width() as usize,
                    self.sps.sub_height() as usize,
                )
            };
            let (pw, ph) = (self.plane_width[c], self.plane_height[c]);
            let depth = if c == 0 {
                self.sps.bit_depth_luma
            } else {
                self.sps.bit_depth_chroma
            };
            let max = (1i32 << depth) - 1;
            for cy in 0..self.ctb_h {
                for cx in 0..self.ctb_w {
                    let params = self.sao[cy * self.ctb_w + cx][c];
                    if params.kind == 0 {
                        continue;
                    }
                    let (x0, y0) = (cx * ctb / sw, cy * ctb / sh);
                    let (x1, y1) = (((cx + 1) * ctb / sw).min(pw), ((cy + 1) * ctb / sh).min(ph));
                    // Which neighbouring CTBs may supply edge samples.
                    let mut usable = [[true; 3]; 3];
                    for (dy, row) in usable.iter_mut().enumerate() {
                        for (dx, cell) in row.iter_mut().enumerate() {
                            let (nx, ny) = (cx as i64 + dx as i64 - 1, cy as i64 + dy as i64 - 1);
                            *cell = self.sao_neighbour(cx, cy, nx, ny);
                        }
                    }
                    let guarded = self.pps.transquant_bypass || self.sps.pcm.is_some();
                    for y in y0..y1 {
                        for x in x0..x1 {
                            if guarded && !self.filterable(x * sw, y * sh) {
                                continue;
                            }
                            let value = i32::from(source[y * pw + x]);
                            let offset = if params.kind == 1 {
                                let band =
                                    ((value >> (depth - 5)) - i32::from(params.band_position)) & 31;
                                if band < 4 {
                                    i32::from(params.offsets[band as usize])
                                } else {
                                    0
                                }
                            } else {
                                let (ax, ay, bx, by) = match params.eo_class {
                                    0 => (-1i64, 0i64, 1i64, 0i64),
                                    1 => (0, -1, 0, 1),
                                    2 => (-1, -1, 1, 1),
                                    _ => (1, -1, -1, 1),
                                };
                                let mut sign_sum = 2;
                                let mut ok = true;
                                for (dx, dy) in [(ax, ay), (bx, by)] {
                                    let (nx, ny) = (x as i64 + dx, y as i64 + dy);
                                    if nx < 0 || ny < 0 || nx >= pw as i64 || ny >= ph as i64 {
                                        ok = false;
                                        break;
                                    }
                                    // The CTB (relative to this one) that holds the neighbour.
                                    let rx = (nx as usize * sw >= (cx + 1) * ctb) as usize + 1
                                        - usize::from((nx as usize * sw) < cx * ctb);
                                    let ry = (ny as usize * sh >= (cy + 1) * ctb) as usize + 1
                                        - usize::from((ny as usize * sh) < cy * ctb);
                                    if !usable[ry][rx] {
                                        ok = false;
                                        break;
                                    }
                                    let neighbour =
                                        i32::from(source[ny as usize * pw + nx as usize]);
                                    sign_sum += (value - neighbour).signum();
                                }
                                if ok {
                                    // Edge categories 1 to 4 map to the offsets in order;
                                    // the flat case (2) has none.
                                    match sign_sum {
                                        0 => i32::from(params.offsets[0]),
                                        1 => i32::from(params.offsets[1]),
                                        3 => i32::from(params.offsets[2]),
                                        4 => i32::from(params.offsets[3]),
                                        _ => 0,
                                    }
                                } else {
                                    0
                                }
                            };
                            if offset != 0 {
                                self.planes[c][y * pw + x] = (value + offset).clamp(0, max) as u16;
                            }
                        }
                    }
                }
            }
        }
    }

    /// Whether samples of CTB (`nx`, `ny`) may be used for the edge offset of a
    /// sample in CTB (`cx`, `cy`).
    fn sao_neighbour(&self, cx: usize, cy: usize, nx: i64, ny: i64) -> bool {
        if nx < 0 || ny < 0 || nx >= self.ctb_w as i64 || ny >= self.ctb_h as i64 {
            return false;
        }
        let (own, other) = (
            self.ctb_slice[cy * self.ctb_w + cx],
            self.ctb_slice[ny as usize * self.ctb_w + nx as usize],
        );
        let (Ok(own), Ok(other)) = (usize::try_from(own), usize::try_from(other)) else {
            return false;
        };
        let (own, other) = (&self.slices[own], &self.slices[other]);
        if own.address == other.address {
            return true;
        }
        // Across slices, the later slice decides.
        let later = if own.address > other.address {
            own
        } else {
            other
        };
        later.loop_filter_across_slices
    }
}
