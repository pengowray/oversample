//! wasm32 with simd128: `i32x4.dot_i16x8_s` does 8 multiply-adds per instruction.

use core::arch::wasm32::*;

/// See [`super::Kernel`].
pub unsafe fn kernel_4x2(a: [*const i16; 4], p: [*const u8; 2], dim: usize) -> [i64; 8] {
    let mut acc = [i32x4_splat(0); 8];
    let mut k = 0;
    while k < dim {
        let y0 = i16x8_extend_low_i8x16(v128_load64_zero(p[0].add(k) as *const u64));
        let y1 = i16x8_extend_low_i8x16(v128_load64_zero(p[1].add(k) as *const u64));
        for (r, ar) in a.iter().enumerate() {
            let x = v128_load(ar.add(k) as *const v128);
            acc[2 * r] = i32x4_add(acc[2 * r], i32x4_dot_i16x8(x, y0));
            acc[2 * r + 1] = i32x4_add(acc[2 * r + 1], i32x4_dot_i16x8(x, y1));
        }
        k += 8;
    }
    acc.map(|v| {
        i32x4_extract_lane::<0>(v) as i64
            + i32x4_extract_lane::<1>(v) as i64
            + i32x4_extract_lane::<2>(v) as i64
            + i32x4_extract_lane::<3>(v) as i64
    })
}
