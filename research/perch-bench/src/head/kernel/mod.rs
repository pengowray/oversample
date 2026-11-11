//! The classifier's inner loop: dot products of 4 int16 rows (spatial positions) with 2
//! int8 rows (prototypes), summed as integers. One version per CPU; each file is only
//! compiled for its own target, so a build carries only the code it runs.
//!
//! | File | Target | Speed (5.8 billion multiply-adds per window) |
//! |---|---|---|
//! | wasm.rs | wasm32 with simd128 | 208 to 214 ms in node |
//! | x86_64.rs | x86-64: SSE2 always, AVX2 when the CPU has it | 201 ms, 157 ms |
//! | portable.rs | anything else (ARM until it has its own) | 3.8 s on x86 |

/// `kernel(a, p, dim)` returns [a0.p0, a0.p1, a1.p0, a1.p1, ..., a3.p1]. Each `a` row is
/// `dim` int16 values and each `p` row `dim` int8 values (as bytes); `dim` is a multiple
/// of 8. Each i32 lane sums at most dim / 4 products of at most 32767 * 127, which fits
/// for dim up to 2048; the lanes are then added as i64.
pub type Kernel = unsafe fn([*const i16; 4], [*const u8; 2], usize) -> [i64; 8];

#[cfg(any(
    test,
    not(any(
        all(target_arch = "wasm32", target_feature = "simd128"),
        target_arch = "x86_64"
    ))
))]
mod portable;
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
mod wasm;
#[cfg(target_arch = "x86_64")]
mod x86_64;

/// The fastest kernel for this CPU and row length.
#[cfg(all(target_arch = "wasm32", target_feature = "simd128"))]
pub fn pick(_dim: usize) -> Kernel {
    wasm::kernel_4x2
}

/// The fastest kernel for this CPU and row length.
#[cfg(target_arch = "x86_64")]
pub fn pick(dim: usize) -> Kernel {
    x86_64::pick(dim)
}

/// The fastest kernel for this CPU and row length.
#[cfg(not(any(
    all(target_arch = "wasm32", target_feature = "simd128"),
    target_arch = "x86_64"
)))]
pub fn pick(_dim: usize) -> Kernel {
    portable::kernel_4x2
}

#[cfg(test)]
mod tests {
    /// The picked kernel (and on x86-64, both versions) gives the same sums as the
    /// portable one, at the extremes too.
    #[test]
    fn matches_portable() {
        let dim = 1536;
        let mut seed = 1u32;
        let mut next = || {
            seed = seed.wrapping_mul(1_664_525).wrapping_add(1_013_904_223);
            seed >> 8
        };
        let mut a: Vec<i16> = (0..4 * dim).map(|_| next() as i16).collect();
        let mut p: Vec<u8> = (0..2 * dim).map(|_| next() as u8).collect();
        a[..dim].fill(i16::MAX);
        p[..dim].fill(127);
        let ap = [0, 1, 2, 3].map(|r| a[r * dim..].as_ptr());
        let pp = [0, 1].map(|r| p[r * dim..].as_ptr());
        #[allow(unused_mut)]
        let mut kernels = vec![super::pick(dim)];
        #[cfg(target_arch = "x86_64")]
        kernels.extend(super::x86_64::all());
        // SAFETY: each pointer starts a full `dim`-long row.
        let want = unsafe { super::portable::kernel_4x2(ap, pp, dim) };
        assert_eq!(want[0], 32767 * 127 * dim as i64);
        for k in kernels {
            assert_eq!(unsafe { k(ap, pp, dim) }, want);
        }
    }
}
