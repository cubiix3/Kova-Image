//! Inverse transforms (8.6.4): the integer DCT of 4 to 32 points and the 4-point DST.
use std::sync::OnceLock;

pub const LEVEL_SCALE: [i32; 6] = [40, 45, 51, 57, 64, 72];

/// The 32-point basis is built from these magnitudes: entry `a` is the value for
/// the angle `a * pi / 64`. Smaller transforms use rows of the 32-point matrix.
const BASE: [i32; 33] = [
    64, 90, 90, 90, 89, 88, 87, 85, 83, 82, 80, 78, 75, 73, 70, 67, 64, 61, 57, 54, 50, 46, 43, 38,
    36, 31, 25, 22, 18, 13, 9, 4, 0,
];
const DST_4: [[i32; 4]; 4] = [
    [29, 55, 74, 84],
    [74, 74, 0, -74],
    [84, -29, -74, 55],
    [55, -84, 74, -29],
];

/// `matrix[k * n + i]`: basis function `k` at position `i`, for size `n`.
fn matrix(n: usize) -> &'static [i32] {
    static MATRICES: OnceLock<[Vec<i32>; 4]> = OnceLock::new();
    let all = MATRICES.get_or_init(|| {
        std::array::from_fn(|index| {
            let n = 4usize << index;
            let step = 32 / n;
            let mut m = vec![0i32; n * n];
            for k in 0..n {
                for i in 0..n {
                    m[k * n + i] = if k == 0 {
                        64
                    } else {
                        // cos(pi * a / 64) with a = k * step * (2 i + 1), folded
                        // into the first quarter wave.
                        let a = (k * step * (2 * i + 1)) % 128;
                        let a = a.min(128 - a);
                        if a > 32 { -BASE[64 - a] } else { BASE[a] }
                    };
                }
            }
            m
        })
    });
    &all[n.trailing_zeros() as usize - 2]
}

/// Inverse transform of the `n` x `n` block `coefficients[y * n + x]`, whose
/// non-zero entries lie within columns `0..=last_x` and rows `0..=last_y`.
/// Writes the residual to `out`.
pub fn inverse(
    coefficients: &[i32],
    n: usize,
    last_x: usize,
    last_y: usize,
    dst: bool,
    bit_depth: u8,
    out: &mut [i32],
) {
    let dst_matrix: [i32; 16];
    let m: &[i32] = if dst {
        dst_matrix = std::array::from_fn(|i| DST_4[i / 4][i % 4]);
        &dst_matrix
    } else {
        matrix(n)
    };
    let mut middle = [0i32; 32 * 32];
    // Columns: for each column x, over the rows.
    for x in 0..=last_x {
        for i in 0..n {
            let mut sum = 0i32;
            for k in 0..=last_y {
                sum += m[k * n + i] * coefficients[k * n + x];
            }
            middle[i * n + x] = ((sum + 64) >> 7).clamp(-32768, 32767);
        }
    }
    // Rows.
    let shift = 20 - u32::from(bit_depth);
    let round = 1i64 << (shift - 1);
    for y in 0..n {
        for i in 0..n {
            let mut sum = 0i64;
            for k in 0..=last_x {
                sum += i64::from(m[k * n + i]) * i64::from(middle[y * n + k]);
            }
            out[y * n + i] = ((sum + round) >> shift) as i32;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_four_point_matrix_is_the_standard_one() {
        assert_eq!(
            matrix(4),
            &[
                64, 64, 64, 64, 83, 36, -36, -83, 64, -64, -64, 64, 36, -83, 83, -36
            ]
        );
    }
    #[test]
    fn the_eight_point_matrix_has_the_standard_rows() {
        let m = matrix(8);
        assert_eq!(&m[8..16], &[89, 75, 50, 18, -18, -50, -75, -89]);
        assert_eq!(&m[16..24], &[83, 36, -36, -83, -83, -36, 36, 83]);
        assert_eq!(&m[24..32], &[75, -18, -89, -50, 50, 89, 18, -75]);
    }
    #[test]
    fn the_thirty_two_point_matrix_starts_like_the_standard() {
        let m = matrix(32);
        assert_eq!(
            &m[32..48],
            &[
                90, 90, 88, 85, 82, 78, 73, 67, 61, 54, 46, 38, 31, 22, 13, 4
            ]
        );
        // Rows are orthogonal up to the scaling (64 * sqrt(32) squared).
        for a in 0..32 {
            for b in 0..a {
                let dot: i64 = (0..32)
                    .map(|i| i64::from(m[a * 32 + i]) * i64::from(m[b * 32 + i]))
                    .sum();
                assert!(dot.abs() < 2000, "rows {a} and {b}: {dot}");
            }
        }
    }
    #[test]
    fn a_dc_coefficient_gives_a_flat_block() {
        for n in [4usize, 8, 16, 32] {
            let mut coefficients = vec![0i32; n * n];
            coefficients[0] = 256;
            let mut out = vec![0i32; n * n];
            inverse(&coefficients, n, 0, 0, false, 8, &mut out);
            let first = out[0];
            assert!(first > 0);
            assert!(out[..n * n].iter().all(|&v| v == first), "n = {n}");
        }
    }
    #[test]
    fn bounding_the_non_zero_region_does_not_change_the_result() {
        let n = 8;
        let mut coefficients = vec![0i32; n * n];
        coefficients[1] = 100;
        coefficients[n + 2] = -57;
        coefficients[2 * n] = 33;
        let mut tight = vec![0i32; n * n];
        let mut full = vec![0i32; n * n];
        inverse(&coefficients, n, 2, 2, false, 8, &mut tight);
        inverse(&coefficients, n, 7, 7, false, 8, &mut full);
        assert_eq!(tight, full);
    }
}
