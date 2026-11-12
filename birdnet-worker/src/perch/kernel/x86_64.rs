//! x86-64: `pmaddwd` (`_mm_madd_epi16`) is the same operation as WASM's
//! `i32x4.dot_i16x8_s`. SSE2 is on every x86-64 CPU; AVX2 does 16 values at a time and is
//! picked at run time when the CPU has it.

use core::arch::x86_64::*;

pub fn pick(dim: usize) -> super::Kernel {
    if dim % 16 == 0 && is_x86_feature_detected!("avx2") {
        kernel_4x2_avx2
    } else {
        kernel_4x2_sse2
    }
}

/// Both versions, for the tests.
#[cfg(test)]
pub fn all() -> Vec<super::Kernel> {
    let mut v: Vec<super::Kernel> = vec![kernel_4x2_sse2];
    if is_x86_feature_detected!("avx2") {
        v.push(kernel_4x2_avx2);
    }
    v
}

/// See [`super::Kernel`].
unsafe fn kernel_4x2_sse2(a: [*const i16; 4], p: [*const u8; 2], dim: usize) -> [i64; 8] {
    let widen = |b: *const u8| {
        let v = _mm_loadl_epi64(b as *const __m128i);
        _mm_srai_epi16::<8>(_mm_unpacklo_epi8(v, v))
    };
    let mut acc = [_mm_setzero_si128(); 8];
    let mut k = 0;
    while k < dim {
        let y0 = widen(p[0].add(k));
        let y1 = widen(p[1].add(k));
        for (r, ar) in a.iter().enumerate() {
            let x = _mm_loadu_si128(ar.add(k) as *const __m128i);
            acc[2 * r] = _mm_add_epi32(acc[2 * r], _mm_madd_epi16(x, y0));
            acc[2 * r + 1] = _mm_add_epi32(acc[2 * r + 1], _mm_madd_epi16(x, y1));
        }
        k += 8;
    }
    acc.map(|v| {
        let mut l = [0i32; 4];
        _mm_storeu_si128(l.as_mut_ptr() as *mut __m128i, v);
        l.iter().map(|&x| x as i64).sum()
    })
}

/// See [`super::Kernel`]; `dim` must be a multiple of 16.
#[target_feature(enable = "avx2")]
unsafe fn kernel_4x2_avx2(a: [*const i16; 4], p: [*const u8; 2], dim: usize) -> [i64; 8] {
    let widen = |b: *const u8| _mm256_cvtepi8_epi16(_mm_loadu_si128(b as *const __m128i));
    let mut acc = [_mm256_setzero_si256(); 8];
    let mut k = 0;
    while k < dim {
        let y0 = widen(p[0].add(k));
        let y1 = widen(p[1].add(k));
        for (r, ar) in a.iter().enumerate() {
            let x = _mm256_loadu_si256(ar.add(k) as *const __m256i);
            acc[2 * r] = _mm256_add_epi32(acc[2 * r], _mm256_madd_epi16(x, y0));
            acc[2 * r + 1] = _mm256_add_epi32(acc[2 * r + 1], _mm256_madd_epi16(x, y1));
        }
        k += 16;
    }
    acc.map(|v| {
        let mut l = [0i32; 8];
        _mm256_storeu_si256(l.as_mut_ptr() as *mut __m256i, v);
        l.iter().map(|&x| x as i64).sum()
    })
}
