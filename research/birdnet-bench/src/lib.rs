//! BirdNET V2.4 (ONNX, 3 s of 48 kHz mono in, 6522 logits out) through tract.
//!
//! The same code builds natively and for wasm32-unknown-unknown. The wasm build
//! exports a tiny C ABI (`alloc`, `load`, `run`) that `bench.mjs` drives from node.

use tract_onnx::prelude::*;

pub const WIN: usize = 144_000;
pub const CLASSES: usize = 6522;

pub type Plan = std::sync::Arc<TypedRunnableModel>;

/// Parse and optimise the model for a fixed batch size.
pub fn load_model(bytes: &[u8], batch: usize) -> TractResult<Plan> {
    // Some conversions (onnxslim with a symbolic batch) store intermediate shapes
    // that clash with a fixed batch; ignore them and let tract infer its own.
    tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, f32::fact([batch, WIN]).into())?
        .into_optimized()?
        .into_runnable()
}

/// Run `batch` windows (batch * WIN samples) and return batch * CLASSES logits.
pub fn run_windows(plan: &Plan, batch: usize, samples: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[batch, WIN], samples)?;
    let out = plan.run(tvec!(input.into()))?;
    Ok(out[0].to_plain_array_view::<f32>()?.iter().copied().collect())
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static PLAN: RefCell<Option<(Plan, usize)>> = const { RefCell::new(None) };
        static ERR: RefCell<String> = const { RefCell::new(String::new()) };
    }

    fn set_err(e: impl std::fmt::Display) -> i32 {
        ERR.with(|s| *s.borrow_mut() = format!("{e:#}"));
        -1
    }

    #[no_mangle]
    pub extern "C" fn alloc(n: usize) -> *mut u8 {
        let mut v = Vec::<u8>::with_capacity(n);
        let p = v.as_mut_ptr();
        std::mem::forget(v);
        p
    }

    #[no_mangle]
    pub extern "C" fn err_ptr() -> *const u8 {
        ERR.with(|s| s.borrow().as_ptr())
    }

    #[no_mangle]
    pub extern "C" fn err_len() -> usize {
        ERR.with(|s| s.borrow().len())
    }

    /// Load the model from `len` bytes at `ptr`. Returns 0 on success.
    #[no_mangle]
    pub extern "C" fn load(ptr: *const u8, len: usize, batch: usize) -> i32 {
        let bytes = unsafe { std::slice::from_raw_parts(ptr, len) };
        match load_model(bytes, batch) {
            Ok(p) => {
                PLAN.with(|s| *s.borrow_mut() = Some((p, batch)));
                0
            }
            Err(e) => set_err(e),
        }
    }

    /// Run one batch: reads batch * WIN f32 at `inp`, writes batch * CLASSES f32 to `out`.
    #[no_mangle]
    pub extern "C" fn run(inp: *const f32, out: *mut f32) -> i32 {
        PLAN.with(|s| {
            let s = s.borrow();
            let Some((plan, batch)) = s.as_ref() else {
                return set_err("no model loaded");
            };
            let samples = unsafe { std::slice::from_raw_parts(inp, batch * WIN) };
            match run_windows(plan, *batch, samples) {
                Ok(v) => {
                    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, v.len()) };
                    0
                }
                Err(e) => set_err(e),
            }
        })
    }
}
