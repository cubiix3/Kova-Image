//! Downscaling of 8-bit interleaved images. Averaging whole blocks first keeps
//! a large reduction cheap and free of aliasing; a two-tap bilinear pass then
//! reaches the exact size. Every step reads and writes plain slices, so a huge
//! photo needs only the source, one small intermediate and the result.
use crate::error::Error;

/// Reduces `source` to `target` (both `(width, height)`), which must not be
/// larger on either axis. `N` is the channel count.
pub fn shrink<const N: usize>(
    pixels: Vec<u8>,
    source: (u32, u32),
    target: (u32, u32),
) -> Result<Vec<u8>, Error> {
    let (width, height) = source;
    let expected = (width as usize)
        .checked_mul(height as usize)
        .and_then(|n| n.checked_mul(N))
        .ok_or(Error::Dimensions)?;
    if pixels.len() != expected || target.0 == 0 || target.1 == 0 {
        return Err(Error::Dimensions);
    }
    if target == source {
        return Ok(pixels);
    }
    if target.0 > width || target.1 > height {
        return Err(Error::Dimensions);
    }
    let k = block_factor(source, target);
    if k == 1 {
        return Ok(bilinear::<N>(&pixels, source, target));
    }
    let (averaged, w, h) = box_shrink::<N>(&pixels, source, k);
    drop(pixels);
    if (w, h) == target {
        Ok(averaged)
    } else {
        Ok(bilinear::<N>(&averaged, (w, h), target))
    }
}

/// Whole-number reduction applied by averaging. The result stays at least as
/// large as the target on both axes, so the bilinear step never exceeds 2:1.
fn block_factor((width, height): (u32, u32), (target_w, target_h): (u32, u32)) -> u32 {
    (width / target_w.max(1))
        .min(height / target_h.max(1))
        .clamp(1, 64)
}

/// Averages `k` by `k` blocks. Edge blocks average the pixels that remain, so
/// the whole picture is kept.
fn box_shrink<const N: usize>(
    src: &[u8],
    (width, height): (u32, u32),
    k: u32,
) -> (Vec<u8>, u32, u32) {
    let (w, h, k) = (width as usize, height as usize, k as usize);
    let (dw, dh) = (w.div_ceil(k), h.div_ceil(k));
    let mut out = vec![0u8; dw * dh * N];
    // k <= 64, so a block sum never exceeds 64 * 64 * 255.
    let mut sums = vec![0u32; dw * N];
    for dy in 0..dh {
        sums.fill(0);
        let (y0, y1) = (dy * k, ((dy + 1) * k).min(h));
        for y in y0..y1 {
            let row = &src[y * w * N..(y + 1) * w * N];
            for (block, sum) in row.chunks(k * N).zip(sums.chunks_exact_mut(N)) {
                for pixel in block.chunks_exact(N) {
                    for (total, value) in sum.iter_mut().zip(pixel) {
                        *total += u32::from(*value);
                    }
                }
            }
        }
        let rows = y1 - y0;
        let out_row = &mut out[dy * dw * N..(dy + 1) * dw * N];
        for (dx, (dst, sum)) in out_row
            .chunks_exact_mut(N)
            .zip(sums.chunks_exact(N))
            .enumerate()
        {
            let count = (rows * (((dx + 1) * k).min(w) - dx * k)) as u32;
            for (value, total) in dst.iter_mut().zip(sum) {
                *value = ((total + count / 2) / count) as u8;
            }
        }
    }
    (out, dw as u32, dh as u32)
}

/// Two source samples per axis with 8-bit fractional weights, sampled at pixel
/// centres. Each output row blends two source rows, then resamples that single
/// row horizontally, so no full-size intermediate exists.
fn bilinear<const N: usize>(
    src: &[u8],
    (width, height): (u32, u32),
    (dw, dh): (u32, u32),
) -> Vec<u8> {
    let (w, h, dw, dh) = (width as usize, height as usize, dw as usize, dh as usize);
    // (first index, second index, weight of the second in 0..=256)
    let taps = |source: usize, target: usize| -> Vec<(usize, usize, u32)> {
        let ratio = source as f64 / target as f64;
        (0..target)
            .map(|i| {
                let position = ((i as f64 + 0.5) * ratio - 0.5).clamp(0.0, (source - 1) as f64);
                let first = position.floor() as usize;
                let second = (first + 1).min(source - 1);
                let weight = ((position - first as f64) * 256.0).round() as u32;
                (first, second, weight)
            })
            .collect()
    };
    let (columns, rows) = (taps(w, dw), taps(h, dh));
    let mut out = vec![0u8; dw * dh * N];
    let mut blended = vec![0u8; w * N];
    for (dy, &(y0, y1, fy)) in rows.iter().enumerate() {
        let upper = &src[y0 * w * N..(y0 + 1) * w * N];
        let lower = &src[y1 * w * N..(y1 + 1) * w * N];
        for ((value, a), b) in blended.iter_mut().zip(upper).zip(lower) {
            *value = ((u32::from(*a) * (256 - fy) + u32::from(*b) * fy + 128) >> 8) as u8;
        }
        let out_row = &mut out[dy * dw * N..(dy + 1) * dw * N];
        for (dst, &(x0, x1, fx)) in out_row.chunks_exact_mut(N).zip(&columns) {
            for (c, value) in dst.iter_mut().enumerate() {
                let a = u32::from(blended[x0 * N + c]);
                let b = u32::from(blended[x1 * N + c]);
                *value = ((a * (256 - fx) + b * fx + 128) >> 8) as u8;
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn blocks_average_and_edges_keep_the_picture() {
        // 5 x 3 pixels, 3 channels, k = 2.
        let mut src = Vec::new();
        for y in 0..3u8 {
            for x in 0..5u8 {
                src.extend_from_slice(&[x * 10 + y, 200, 7]);
            }
        }
        let (out, w, h) = box_shrink::<3>(&src, (5, 3), 2);
        assert_eq!((w, h), (3, 2));
        // First block (x 0..2, y 0..2): (0 + 10 + 1 + 11) / 4 = 5.5, rounded up.
        assert_eq!(&out[..3], &[6, 200, 7]);
        // Last column holds only x = 4: (40 + 41) / 2 = 40.5, rounded up.
        assert_eq!(&out[6..9], &[41, 200, 7]);
        // Last row holds only y = 2: (2 + 12) / 2 = 7.
        assert_eq!(out[9], 7);
        assert_eq!(block_factor((6000, 4000), (1920, 1280)), 3);
        assert_eq!(block_factor((1000, 1000), (900, 900)), 1);
    }
    #[test]
    fn flat_and_exact_cases_are_preserved() {
        let flat = vec![123u8; 40 * 30 * 4];
        for target in [(20, 15), (13, 9), (39, 29), (1, 1)] {
            let out = shrink::<4>(flat.clone(), (40, 30), target).unwrap();
            assert_eq!(out.len(), target.0 as usize * target.1 as usize * 4);
            assert!(out.iter().all(|v| *v == 123), "{target:?}");
        }
        let same = shrink::<3>(vec![1, 2, 3], (1, 1), (1, 1)).unwrap();
        assert_eq!(same, vec![1, 2, 3]);
        assert_eq!(
            shrink::<3>(vec![0; 12], (2, 2), (3, 3)),
            Err(Error::Dimensions)
        );
        assert_eq!(
            shrink::<3>(vec![0; 11], (2, 2), (1, 1)),
            Err(Error::Dimensions)
        );
    }
    #[test]
    fn result_stays_close_to_the_image_crate_triangle_filter() {
        // Smooth pattern with some texture, reduced by several ratios.
        let (w, h) = (640u32, 480u32);
        let image = image::RgbImage::from_fn(w, h, |x, y| {
            let n = (x.wrapping_mul(2654435761) ^ y.wrapping_mul(40503)) % 23;
            image::Rgb([
                (x * 255 / w) as u8 / 2 + n as u8,
                (y * 255 / h) as u8 / 2 + n as u8,
                ((x + y) * 255 / (w + h)) as u8,
            ])
        });
        for target in [(320, 240), (211, 158), (97, 73), (600, 450)] {
            let ours = shrink::<3>(image.as_raw().clone(), (w, h), target).unwrap();
            let reference = image::imageops::resize(
                &image,
                target.0,
                target.1,
                image::imageops::FilterType::Triangle,
            );
            let mean = ours
                .iter()
                .zip(reference.as_raw())
                .map(|(a, b)| f64::from(a.abs_diff(*b)))
                .sum::<f64>()
                / ours.len() as f64;
            assert!(mean < 3.0, "{target:?}: mean difference {mean}");
        }
    }
}
