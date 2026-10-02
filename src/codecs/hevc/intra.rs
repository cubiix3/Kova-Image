//! Intra prediction (8.4.4.2): reference samples, their filtering, and the
//! planar, DC and angular predictors.
use super::picture::Picture;

const ANGLE: [i32; 35] = [
    0, 0, 32, 26, 21, 17, 13, 9, 5, 2, 0, -2, -5, -9, -13, -17, -21, -26, -32, -26, -21, -17, -13,
    -9, -5, -2, 0, 2, 5, 9, 13, 17, 21, 26, 32,
];
/// For modes 11 to 25.
const INVERSE_ANGLE: [i32; 15] = [
    -4096, -1638, -910, -630, -482, -390, -315, -256, -315, -390, -482, -630, -910, -1638, -4096,
];

/// 4:2:2 chroma uses another prediction direction than the luma-derived mode
/// says, since its samples are not square (Table 8-3).
pub const MODE_422: [u8; 35] = [
    0, 1, 2, 2, 2, 2, 3, 5, 7, 8, 10, 12, 13, 15, 17, 18, 19, 20, 21, 22, 23, 23, 24, 24, 25, 25,
    26, 27, 27, 28, 28, 29, 29, 30, 31,
];

impl Picture<'_> {
    /// Predicts the `n` x `n` block of component `c` at (`x0`, `y0`) in that
    /// component's samples and writes it into the picture.
    ///
    /// `disable_boundary_filter` is set for lossless blocks with implicit RDPCM.
    #[allow(clippy::too_many_arguments)]
    pub fn predict_intra(
        &mut self,
        slice_address: u32,
        c: usize,
        x0: usize,
        y0: usize,
        n: usize,
        mode: u8,
        disable_boundary_filter: bool,
    ) {
        let depth = if c == 0 {
            self.sps.bit_depth_luma
        } else {
            self.sps.bit_depth_chroma
        };
        let (sw, sh) = if c == 0 {
            (1, 1)
        } else {
            (
                self.sps.sub_width() as usize,
                self.sps.sub_height() as usize,
            )
        };
        let stride = self.plane_width[c];
        let mut border = [0i32; 4 * 32 + 1];
        let mut known = [false; 4 * 32 + 1];
        let corner = 2 * n;
        // Availability is decided per run of samples that cover one 4x4 luma block.
        let run_v = (4 / sh).max(1);
        let run_h = (4 / sw).max(1);
        let luma = |cx: i64, cy: i64| (cx * sw as i64, cy * sh as i64);
        // The left column, bottom to top, and the corner.
        for y in (0..2 * n).step_by(run_v) {
            let (lx, ly) = luma(x0 as i64 - 1, (y0 + y) as i64);
            if self.available(slice_address, lx, ly) {
                for yy in y..(y + run_v).min(2 * n) {
                    border[corner - 1 - yy] =
                        i32::from(self.planes[c][(y0 + yy) * stride + x0 - 1]);
                    known[corner - 1 - yy] = true;
                }
            }
        }
        let (lx, ly) = luma(x0 as i64 - 1, y0 as i64 - 1);
        if self.available(slice_address, lx, ly) {
            border[corner] = i32::from(self.planes[c][(y0 - 1) * stride + x0 - 1]);
            known[corner] = true;
        }
        for x in (0..2 * n).step_by(run_h) {
            let (lx, ly) = luma((x0 + x) as i64, y0 as i64 - 1);
            if self.available(slice_address, lx, ly) {
                for xx in x..(x + run_h).min(2 * n) {
                    border[corner + 1 + xx] =
                        i32::from(self.planes[c][(y0 - 1) * stride + x0 + xx]);
                    known[corner + 1 + xx] = true;
                }
            }
        }
        // Substitution (8.4.4.2.2): copy from the nearest available sample, going
        // from the bottom left over the corner to the top right.
        let total = 4 * n + 1;
        match known[..total].iter().position(|&k| k) {
            None => border[..total].fill(1 << (depth - 1)),
            Some(first) => {
                if first > 0 {
                    let value = border[first];
                    border[..first].fill(value);
                }
                for i in first + 1..total {
                    if !known[i] {
                        border[i] = border[i - 1];
                    }
                }
            }
        }

        // Filtering of the reference samples (8.4.4.2.3).
        let filtered = !self.sps.range.intra_smoothing_disabled
            && (c == 0 || self.sps.chroma_array_type == 3)
            && mode != 1
            && n != 4
            && {
                let distance = (i32::from(mode) - 26)
                    .abs()
                    .min((i32::from(mode) - 10).abs());
                distance
                    > match n {
                        8 => 7,
                        16 => 1,
                        _ => 0,
                    }
            };
        if filtered {
            let strong = self.sps.strong_intra_smoothing
                && c == 0
                && n == 32
                && (border[corner] + border[corner + 64] - 2 * border[corner + 32]).abs()
                    < (1 << (depth - 5))
                && (border[corner] + border[corner - 64] - 2 * border[corner - 32]).abs()
                    < (1 << (depth - 5));
            let original = border;
            if strong {
                for i in 1..=63i32 {
                    let lower = original[corner - 64] - original[corner];
                    let upper = original[corner + 64] - original[corner];
                    border[corner - i as usize] = original[corner] + ((i * lower + 32) >> 6);
                    border[corner + i as usize] = original[corner] + ((i * upper + 32) >> 6);
                }
            } else {
                for i in 1..total - 1 {
                    border[i] = (original[i - 1] + 2 * original[i] + original[i + 1] + 2) >> 2;
                }
            }
        }

        let max = (1i32 << depth) - 1;
        let left = |y: usize| border[corner - 1 - y];
        let top = |x: usize| border[corner + 1 + x];
        let base = y0 * stride + x0;
        let plane = &mut self.planes[c];
        match mode {
            0 => {
                // Planar.
                let shift = n.trailing_zeros() + 1;
                let (top_right, bottom_left) = (top(n), left(n));
                for y in 0..n {
                    for x in 0..n {
                        let value = (n - 1 - x) as i32 * left(y)
                            + (x + 1) as i32 * top_right
                            + (n - 1 - y) as i32 * top(x)
                            + (y + 1) as i32 * bottom_left
                            + n as i32;
                        plane[base + y * stride + x] = (value >> shift) as u16;
                    }
                }
            }
            1 => {
                // DC, with smoothed edges for small luma blocks.
                let sum: i32 = (0..n).map(|i| top(i) + left(i)).sum();
                let dc = (sum + n as i32) >> (n.trailing_zeros() + 1);
                for y in 0..n {
                    plane[base + y * stride..base + y * stride + n].fill(dc as u16);
                }
                if c == 0 && n < 32 {
                    plane[base] = ((left(0) + 2 * dc + top(0) + 2) >> 2) as u16;
                    for x in 1..n {
                        plane[base + x] = ((top(x) + 3 * dc + 2) >> 2) as u16;
                    }
                    for y in 1..n {
                        plane[base + y * stride] = ((left(y) + 3 * dc + 2) >> 2) as u16;
                    }
                }
            }
            _ => {
                let angle = ANGLE[mode as usize];
                let vertical = mode >= 18;
                // reference[offset + i] holds the reference sample ref[i], i from -n to 2n.
                let offset = n as i32;
                let mut reference = [0i32; 3 * 32 + 1];
                // The main side: from the corner outwards.
                for i in 0..=n {
                    reference[offset as usize + i] = if vertical {
                        border[corner + i]
                    } else {
                        border[corner - i]
                    };
                }
                if angle < 0 {
                    let last = (n as i32 * angle) >> 5;
                    if last < -1 {
                        let inverse = INVERSE_ANGLE[usize::from(mode) - 11];
                        for i in last..=-1 {
                            let k = ((i * inverse + 128) >> 8) as usize;
                            reference[(offset + i) as usize] = if vertical {
                                border[corner - k]
                            } else {
                                border[corner + k]
                            };
                        }
                    }
                } else {
                    for i in n + 1..=2 * n {
                        reference[offset as usize + i] = if vertical {
                            border[corner + i]
                        } else {
                            border[corner - i]
                        };
                    }
                }
                // Outer index runs over the angle steps, inner along the side.
                for outer in 0..n {
                    let step = (outer as i32 + 1) * angle;
                    let index = step >> 5;
                    let fraction = step & 31;
                    for inner in 0..n {
                        let at = (offset + inner as i32 + index + 1) as usize;
                        let value = if fraction != 0 {
                            ((32 - fraction) * reference[at] + fraction * reference[at + 1] + 16)
                                >> 5
                        } else {
                            reference[at]
                        };
                        let (x, y) = if vertical {
                            (inner, outer)
                        } else {
                            (outer, inner)
                        };
                        plane[base + y * stride + x] = value as u16;
                    }
                }
                let edge = c == 0 && n < 32 && !disable_boundary_filter;
                if edge && mode == 26 {
                    for y in 0..n {
                        let value = top(0) + ((left(y) - border[corner]) >> 1);
                        plane[base + y * stride] = value.clamp(0, max) as u16;
                    }
                } else if edge && mode == 10 {
                    for x in 0..n {
                        let value = left(0) + ((top(x) - border[corner]) >> 1);
                        plane[base + x] = value.clamp(0, max) as u16;
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::codecs::hevc::params::{Pps, RangeExtension, Sps};

    pub fn sps(width: u32, height: u32) -> Sps {
        Sps {
            id: 0,
            separate_colour_planes: false,
            chroma_array_type: 1,
            width,
            height,
            crop: [0; 4],
            bit_depth_luma: 8,
            bit_depth_chroma: 8,
            num_short_term_ref_pic_sets: 0,
            rps_sizes: Vec::new(),
            log2_poc_lsb: 4,
            long_term_ref_pics_present: false,
            num_long_term_ref_pics_sps: 0,
            temporal_mvp: false,
            log2_min_cb: 3,
            log2_ctb: 4,
            log2_min_tb: 2,
            log2_max_tb: 4,
            max_transform_hierarchy_depth_intra: 1,
            scaling_lists: None,
            sao: false,
            pcm: None,
            strong_intra_smoothing: true,
            vui: None,
            range: RangeExtension::default(),
        }
    }
    pub fn pps() -> Pps {
        super::super::params::test_pps()
    }

    #[test]
    fn the_inverse_angle_table_matches_the_angles() {
        // invAngle = round(8192 / angle) for the modes that need it.
        for mode in 11..=25usize {
            let angle = ANGLE[mode];
            let expected = (8192.0 / f64::from(angle)).round() as i32;
            assert_eq!(INVERSE_ANGLE[mode - 11], expected, "mode {mode}");
        }
    }

    #[test]
    fn an_isolated_block_predicts_the_mid_grey_in_every_mode() {
        let sps = sps(16, 16);
        let pps = pps();
        for mode in 0..35 {
            let mut picture = Picture::new(&sps, &pps);
            picture.planes[0].fill(0);
            picture.predict_intra(0, 0, 0, 0, 8, mode, false);
            for y in 0..8 {
                for x in 0..8 {
                    assert_eq!(picture.planes[0][y * 16 + x], 128, "mode {mode}");
                }
            }
        }
    }
}
