//! Perch v2's classifier (a ProtoPNet head) in plain Rust, on the int8 file from
//! export_head.py. Input is the spatial embedding (16 x 4 positions of 1536 values) from
//! the embedding model; output is the 14,795 label logits.
//!
//!   s = each position / (its L2 norm + 1e-5)
//!   m[c, k] = max over positions of s . P[:, c*4 + k]      (P = q * scale)
//!   logit[c] = sum over k of W[c, k] * m[c, k] + b[c]
//!
//! The int8 prototypes stay int8 in memory (91 MB) and are multiplied as integers. They
//! are read a block at a time, and each block is used for all 64 positions before the
//! next one, so the block stays in cache. The inner loop is in [`kernel`], one file per
//! CPU.

mod kernel;

/// Prototypes per block (BLOCK * 1536 bytes).
const BLOCK: usize = 32;
const HEADER: usize = 8 + 12;

pub struct Head {
    pub dim: usize,
    pub classes: usize,
    protos: usize,
    /// W * scale, per prototype column.
    ws: Vec<f32>,
    b: Vec<f32>,
    /// The whole file; the int8 prototypes start at `q_off`.
    bytes: Vec<u8>,
    q_off: usize,
}

fn f32s(b: &[u8]) -> Vec<f32> {
    b.chunks_exact(4)
        .map(|c| f32::from_le_bytes(c.try_into().unwrap()))
        .collect()
}

impl Head {
    pub fn from_bytes(bytes: Vec<u8>) -> Result<Head, String> {
        if bytes.len() < HEADER || &bytes[..8] != b"PERCHHD1" {
            return Err("not a Perch classifier file (PERCHHD1)".into());
        }
        let u = |i: usize| {
            u32::from_le_bytes(bytes[8 + 4 * i..12 + 4 * i].try_into().unwrap()) as usize
        };
        let (dim, classes, protos) = (u(0), u(1), u(2));
        let cols = classes * protos;
        let q_off = HEADER + 4 * (2 * cols + classes);
        if dim % 8 != 0 || bytes.len() != q_off + cols * dim {
            return Err(format!(
                "classifier file is {} bytes, expected {}",
                bytes.len(),
                q_off + cols * dim
            ));
        }
        let scale = f32s(&bytes[HEADER..HEADER + 4 * cols]);
        let w = f32s(&bytes[HEADER + 4 * cols..HEADER + 8 * cols]);
        let b = f32s(&bytes[HEADER + 8 * cols..q_off]);
        let ws = w.iter().zip(&scale).map(|(w, s)| w * s).collect();
        Ok(Head {
            dim,
            classes,
            protos,
            ws,
            b,
            bytes,
            q_off,
        })
    }

    /// `spatial` is positions * dim values. Returns `classes` logits.
    ///
    /// Each normalised position is stored as i16 with its own scale, so the dot products
    /// are i16 x i8 sums in i32 (see [`kernel::Kernel`] for why dim is at most 2048).
    pub fn scores(&self, spatial: &[f32]) -> Vec<f32> {
        let dim = self.dim;
        assert!(dim <= 2048);
        let kernel = kernel::pick(dim);
        let n = spatial.len() / dim;
        let mut v = vec![0i16; n * dim];
        let mut vs = vec![0f32; n];
        for (i, row) in spatial[..n * dim].chunks(dim).enumerate() {
            let norm = row.iter().map(|x| x * x).sum::<f32>().sqrt() + 1e-5;
            let big = row
                .iter()
                .fold(0f32, |m, x| m.max(x.abs()))
                .max(f32::MIN_POSITIVE);
            for (d, x) in v[i * dim..][..dim].iter_mut().zip(row) {
                *d = (x * (32767.0 / big)).round() as i16;
            }
            vs[i] = big / 32767.0 / norm;
        }
        let cols = self.classes * self.protos;
        let q = &self.bytes[self.q_off..];
        let mut best = vec![f32::NEG_INFINITY; cols];
        for c0 in (0..cols).step_by(BLOCK) {
            let nb = BLOCK.min(cols - c0);
            max_dots(
                kernel,
                &v,
                &vs,
                &q[c0 * dim..(c0 + nb) * dim],
                nb,
                dim,
                &mut best[c0..c0 + nb],
            );
        }
        (0..self.classes)
            .map(|c| {
                let k = c * self.protos..(c + 1) * self.protos;
                self.b[c]
                    + self.ws[k.clone()]
                        .iter()
                        .zip(&best[k])
                        .map(|(w, m)| w * m)
                        .sum::<f32>()
            })
            .collect()
    }
}

/// best[j] = max(best[j], max over rows i of as[i] * (a[i] . p[j])), for the rows of a
/// (i16, scale `as`) and m rows of p (int8, as bytes), each `dim` long. Works on 4 rows of
/// a and 2 of p at a time; a short last group repeats its last row, which doesn't change
/// a max.
fn max_dots(
    kernel: kernel::Kernel,
    a: &[i16],
    a_scale: &[f32],
    p: &[u8],
    m: usize,
    dim: usize,
    best: &mut [f32],
) {
    let n = a_scale.len();
    let pr = |j: usize| p[j.min(m - 1) * dim..][..dim].as_ptr();
    let ar = |i: usize| i.min(n - 1);
    for j in (0..m).step_by(2) {
        let pp = [pr(j), pr(j + 1)];
        for i in (0..n).step_by(4) {
            let rows = [ar(i), ar(i + 1), ar(i + 2), ar(i + 3)];
            let aa = rows.map(|r| a[r * dim..][..dim].as_ptr());
            // SAFETY: every pointer is the start of a full `dim`-long row inside `a` or `p`.
            let d = unsafe { kernel(aa, pp, dim) };
            for (r, &row) in rows.iter().enumerate() {
                best[j] = best[j].max(d[2 * r] as f32 * a_scale[row]);
                if j + 1 < m {
                    best[j + 1] = best[j + 1].max(d[2 * r + 1] as f32 * a_scale[row]);
                }
            }
        }
    }
}
