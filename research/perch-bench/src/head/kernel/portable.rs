//! Any other CPU, and the reference the tests check the others against. The compiler did
//! not vectorise this on x86 (3.8 s per window, against 157 to 214 ms for the others).

/// See [`super::Kernel`].
pub unsafe fn kernel_4x2(a: [*const i16; 4], p: [*const u8; 2], dim: usize) -> [i64; 8] {
    let a = a.map(|r| std::slice::from_raw_parts(r, dim));
    let p = p.map(|r| std::slice::from_raw_parts(r, dim));
    let mut acc = [[0i32; 8]; 8];
    for k in (0..dim).step_by(8) {
        for r in 0..4 {
            for c in 0..2 {
                for l in 0..8 {
                    acc[2 * r + c][l] += a[r][k + l] as i32 * p[c][k + l] as i8 as i32;
                }
            }
        }
    }
    acc.map(|v| v.iter().map(|&x| x as i64).sum())
}
