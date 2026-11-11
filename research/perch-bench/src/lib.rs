//! Perch v2 (ONNX, 5 s of 32 kHz mono in) through tract.
//!
//! Works with the full model (outputs embedding, spatial_embedding, spectrogram, label),
//! the embedding-only cut from split.py (embedding, spatial_embedding) and the tphakala
//! regional slices. `run_windows` returns the 1536-value embedding followed by the label
//! logits when the model has them.
//!
//! The same code builds natively and for wasm32. The wasm build exports a tiny C ABI
//! (`alloc`, `load`, `out_len`, `run`) that `bench.mjs` drives from node.

use tract_onnx::prelude::*;

pub const WIN: usize = 160_000;
pub const EMB: usize = 1536;

pub struct Model {
    pub plan: std::sync::Arc<TypedRunnableModel>,
    /// Number of label logits (0 for the embedding-only model).
    pub classes: usize,
}

/// Parse and optimise the model for a fixed batch size. Only `embedding` and (if
/// present) `label` are kept as outputs, so tract drops the spectrogram and
/// spatial_embedding outputs.
pub fn load_model(bytes: &[u8], batch: usize) -> TractResult<Model> {
    let mut m = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, f32::fact([batch, WIN]).into())?;
    let outlets = m.output_outlets()?.to_vec();
    let mut keep = vec![];
    for want in ["embedding", "label"] {
        if let Some(o) = outlets.iter().find(|o| m.outlet_label(**o) == Some(want)) {
            keep.push(*o);
        }
    }
    if keep.is_empty() {
        let names: Vec<_> = outlets.iter().map(|o| m.outlet_label(*o)).collect();
        return Err(tract_onnx::tract_core::internal::format_err!("no embedding or label output among {names:?}"));
    }
    m.select_output_outlets(&keep)?;
    let typed = m.into_typed()?.into_decluttered()?;
    let classes = if keep.len() > 1 {
        let fact = typed.outlet_fact(typed.output_outlets()?[1])?;
        fact.shape.as_concrete().map_or(0, |s| s[1])
    } else {
        0
    };
    let plan = typed.into_optimized()?.into_runnable()?;
    Ok(Model { plan, classes })
}

impl Model {
    pub fn out_len(&self) -> usize {
        EMB + self.classes
    }
}

/// Run `batch` windows (batch * WIN samples). Returns, per window, EMB embedding
/// values followed by `classes` logits.
pub fn run_windows(m: &Model, batch: usize, samples: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[batch, WIN], samples)?;
    let out = m.plan.run(tvec!(input.into()))?;
    let emb = out[0].to_plain_array_view::<f32>()?;
    let lab = if out.len() > 1 { Some(out[1].to_plain_array_view::<f32>()?) } else { None };
    let mut v = Vec::with_capacity(batch * m.out_len());
    for b in 0..batch {
        v.extend(emb.index_axis(tract_ndarray::Axis(0), b).iter().copied());
        if let Some(l) = &lab {
            v.extend(l.index_axis(tract_ndarray::Axis(0), b).iter().copied());
        }
    }
    Ok(v)
}

/// wasm32-unknown-unknown (the app's browser target), through wasm-bindgen.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod bindgen {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct Perch(Model);

    #[wasm_bindgen]
    impl Perch {
        #[wasm_bindgen(constructor)]
        pub fn new(model: &[u8]) -> Result<Perch, JsError> {
            load_model(model, 1)
                .map(Perch)
                .map_err(|e| JsError::new(&format!("{e:#}")))
        }

        pub fn out_len(&self) -> usize {
            self.0.out_len()
        }

        /// One 5 s window (160000 samples) in; embedding then logits out.
        pub fn run(&self, window: &[f32]) -> Result<Vec<f32>, JsError> {
            run_windows(&self.0, 1, window).map_err(|e| JsError::new(&format!("{e:#}")))
        }
    }
}

#[cfg(target_arch = "wasm32")]
mod wasm {
    use super::*;
    use std::cell::RefCell;

    thread_local! {
        static MODEL: RefCell<Option<(Model, usize)>> = const { RefCell::new(None) };
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

    /// Free a buffer from `alloc` (used to drop the model bytes after loading).
    #[no_mangle]
    pub extern "C" fn dealloc(p: *mut u8, n: usize) {
        unsafe { drop(Vec::from_raw_parts(p, 0, n)) };
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
            Ok(m) => {
                MODEL.with(|s| *s.borrow_mut() = Some((m, batch)));
                0
            }
            Err(e) => set_err(e),
        }
    }

    /// f32 values written per window by `run`.
    #[no_mangle]
    pub extern "C" fn out_len() -> usize {
        MODEL.with(|s| s.borrow().as_ref().map_or(0, |(m, _)| m.out_len()))
    }

    /// Run one batch: reads batch * WIN f32 at `inp`, writes batch * out_len() f32 to `out`.
    #[no_mangle]
    pub extern "C" fn run(inp: *const f32, out: *mut f32) -> i32 {
        MODEL.with(|s| {
            let s = s.borrow();
            let Some((m, batch)) = s.as_ref() else {
                return set_err("no model loaded");
            };
            let samples = unsafe { std::slice::from_raw_parts(inp, batch * WIN) };
            match run_windows(m, *batch, samples) {
                Ok(v) => {
                    unsafe { std::ptr::copy_nonoverlapping(v.as_ptr(), out, v.len()) };
                    0
                }
                Err(e) => set_err(e),
            }
        })
    }
}
