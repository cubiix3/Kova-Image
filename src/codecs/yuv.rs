//! Conversion of decoded YUV pictures (AV1 and HEVC) to 8-bit RGBA.
use crate::{error::Error, security};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Layout {
    Mono,
    Yuv420,
    Yuv422,
    Yuv444,
}

/// A picture made of up to three planes of samples.
pub trait Planar {
    fn size(&self) -> (usize, usize);
    fn layout(&self) -> Layout;
    /// Bits per sample (8 to 12), the same for every plane.
    fn depth(&self) -> u32;
    /// One row of plane `index` (0 luma, 1 and 2 chroma) as 16-bit samples.
    fn row(&self, index: usize, y: usize, out: &mut [u16]);
}

/// Width and height of plane `index` of a picture.
pub fn plane_size(layout: Layout, (w, h): (usize, usize), index: usize) -> (usize, usize) {
    if index == 0 {
        return (w, h);
    }
    match layout {
        Layout::Mono => (0, 0),
        Layout::Yuv420 => (w.div_ceil(2), h.div_ceil(2)),
        Layout::Yuv422 => (w.div_ceil(2), h),
        Layout::Yuv444 => (w, h),
    }
}

/// Opaque RGBA from the luma and chroma planes. Half-size chroma is spread to
/// full size with the usual 3:1 linear weights.
pub fn to_rgba<P: Planar>(picture: &P, matrix: u16, full_range: bool) -> Result<Vec<u8>, Error> {
    let size = picture.size();
    let (w, h) = size;
    let layout = picture.layout();
    let mut out = vec![0u8; security::rgba_bytes(w as u32, h as u32)?];
    let depth = picture.depth();
    let k = Coefficients::new(matrix, full_range, depth)?;
    let identity = matrix == 0;
    if identity && layout != Layout::Yuv444 {
        return Err(Error::Unsupported);
    }
    let (cw, ch) = plane_size(layout, size, 1);
    let mut luma = vec![0u16; w];
    let mut near = vec![0u16; cw];
    let mut far = vec![0u16; cw];
    // Chroma of the current row, one value per chroma column.
    let mut cb = vec![0u16; cw];
    let mut cr = vec![0u16; cw];
    // Chroma of the current row at full width.
    let mut blue = vec![k.mid; w];
    let mut red = vec![k.mid; w];
    let shift = depth - 8;
    let to8 = |v: i32| -> u8 {
        if shift == 0 {
            v.clamp(0, 255) as u8
        } else {
            ((v + (1 << (shift - 1))) >> shift).clamp(0, 255) as u8
        }
    };
    for y in 0..h {
        picture.row(0, y, &mut luma);
        if layout != Layout::Mono {
            for (plane, row) in [(1, &mut cb), (2, &mut cr)] {
                if layout == Layout::Yuv420 {
                    let c = y / 2;
                    let other = if y % 2 == 0 {
                        c.saturating_sub(1)
                    } else {
                        (c + 1).min(ch - 1)
                    };
                    picture.row(plane, c, &mut near);
                    picture.row(plane, other, &mut far);
                    for x in 0..cw {
                        row[x] = ((3 * u32::from(near[x]) + u32::from(far[x]) + 2) >> 2) as u16;
                    }
                } else {
                    picture.row(plane, y, row);
                }
            }
            for (full, half) in [(&mut blue, &cb), (&mut red, &cr)] {
                for (x, value) in full.iter_mut().enumerate() {
                    *value = if layout == Layout::Yuv444 {
                        i32::from(half[x])
                    } else {
                        let i = x / 2;
                        let j = if x % 2 == 0 {
                            i.saturating_sub(1)
                        } else {
                            (i + 1).min(cw - 1)
                        };
                        (3 * i32::from(half[i]) + i32::from(half[j]) + 2) >> 2
                    };
                }
            }
        }
        let row = &mut out[y * w * 4..(y + 1) * w * 4];
        for x in 0..w {
            let (r, g, b) = if identity {
                // GBR: the "luma" plane carries green, "U" blue, "V" red.
                (red[x], i32::from(luma[x]), blue[x])
            } else {
                k.rgb(i32::from(luma[x]), blue[x], red[x])
            };
            row[x * 4..x * 4 + 4].copy_from_slice(&[to8(r), to8(g), to8(b), 255]);
        }
    }
    Ok(out)
}

/// The luma plane as 8-bit values, with limited range expanded. This is how an
/// alpha channel is stored: as a monochrome picture.
pub fn to_alpha<P: Planar>(picture: &P, full_range: bool) -> Vec<u8> {
    let (w, h) = picture.size();
    let mut out = vec![0u8; w * h];
    let mut luma = vec![0u16; w];
    let depth = picture.depth();
    let max = (1i32 << depth) - 1;
    let (low, high) = (16 << (depth - 8), 235 << (depth - 8));
    for y in 0..h {
        picture.row(0, y, &mut luma);
        for (o, &v) in out[y * w..(y + 1) * w].iter_mut().zip(&luma) {
            let v = i32::from(v);
            let scaled = if full_range {
                (v * 255 + max / 2) / max
            } else {
                ((v - low).clamp(0, high - low) * 255 + (high - low) / 2) / (high - low)
            };
            *o = scaled as u8;
        }
    }
    out
}

/// Fixed-point (16 fractional bits) YUV to RGB, in the picture's own bit depth.
struct Coefficients {
    luma_offset: i32,
    mid: i32,
    y: i32,
    r_v: i32,
    g_u: i32,
    g_v: i32,
    b_u: i32,
}
impl Coefficients {
    fn new(matrix: u16, full_range: bool, depth: u32) -> Result<Self, Error> {
        // Weights of red and blue in luma.
        let (kr, kb): (f64, f64) = match matrix {
            // Identity (GBR) needs no matrix; any values do.
            0 | 1 => (0.2126, 0.0722),
            4 => (0.30, 0.11),
            5 | 6 | 2 => (0.299, 0.114),
            7 => (0.212, 0.087),
            9 | 10 => (0.2627, 0.0593),
            _ => return Err(Error::Unsupported),
        };
        let kg = 1.0 - kr - kb;
        let (y_scale, c_scale) = if full_range {
            (1.0, 1.0)
        } else {
            (255.0 / 219.0, 255.0 / 224.0)
        };
        let fixed = |v: f64| (v * 65536.0).round() as i32;
        let up = 1 << (depth - 8);
        Ok(Self {
            luma_offset: if full_range { 0 } else { 16 * up },
            mid: 128 * up,
            y: fixed(y_scale),
            r_v: fixed(2.0 * (1.0 - kr) * c_scale),
            b_u: fixed(2.0 * (1.0 - kb) * c_scale),
            g_u: fixed(-2.0 * (1.0 - kb) * kb / kg * c_scale),
            g_v: fixed(-2.0 * (1.0 - kr) * kr / kg * c_scale),
        })
    }
    /// Red, green and blue in the picture's bit depth (unclamped).
    fn rgb(&self, y: i32, u: i32, v: i32) -> (i32, i32, i32) {
        let luma = i64::from(y - self.luma_offset) * i64::from(self.y);
        let (u, v) = (i64::from(u - self.mid), i64::from(v - self.mid));
        let round = 1i64 << 15;
        let r = (luma + v * i64::from(self.r_v) + round) >> 16;
        let g = (luma + u * i64::from(self.g_u) + v * i64::from(self.g_v) + round) >> 16;
        let b = (luma + u * i64::from(self.b_u) + round) >> 16;
        (r as i32, g as i32, b as i32)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct Grey {
        value: u16,
    }
    impl Planar for Grey {
        fn size(&self) -> (usize, usize) {
            (4, 2)
        }
        fn layout(&self) -> Layout {
            Layout::Yuv420
        }
        fn depth(&self) -> u32 {
            8
        }
        fn row(&self, index: usize, _y: usize, out: &mut [u16]) {
            out.fill(if index == 0 { self.value } else { 128 });
        }
    }

    #[test]
    fn limited_range_black_and_white_map_to_the_ends() {
        let black = to_rgba(&Grey { value: 16 }, 1, false).unwrap();
        assert_eq!(&black[..4], &[0, 0, 0, 255]);
        let white = to_rgba(&Grey { value: 235 }, 1, false).unwrap();
        assert_eq!(&white[..4], &[255, 255, 255, 255]);
        let mid = to_rgba(&Grey { value: 128 }, 1, true).unwrap();
        assert_eq!(&mid[..4], &[128, 128, 128, 255]);
    }
    #[test]
    fn unknown_matrices_are_refused() {
        assert!(to_rgba(&Grey { value: 16 }, 99, false).is_err());
    }
}
