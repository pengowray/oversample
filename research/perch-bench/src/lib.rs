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

pub mod head;

pub const WIN: usize = 160_000;
pub const EMB: usize = 1536;

pub struct Model {
    pub plan: std::sync::Arc<TypedRunnableModel>,
    /// Values of the spatial embedding per window (0 unless asked for).
    pub spatial: usize,
    /// Number of label logits (0 for the embedding-only model).
    pub classes: usize,
}

/// Parse and optimise the model for a fixed batch size. Only `embedding` and (if
/// present) `label` are kept as outputs, so tract drops the spectrogram and
/// spatial_embedding outputs.
pub fn load_model(bytes: &[u8], batch: usize) -> TractResult<Model> {
    load_model_with(bytes, batch, false)
}

/// As `load_model`; with `spatial`, also keeps `spatial_embedding` (the classifier's
/// input, see head.rs), returned between the embedding and the logits.
pub fn load_model_with(bytes: &[u8], batch: usize, spatial: bool) -> TractResult<Model> {
    let mut m = tract_onnx::onnx()
        .with_ignore_value_info(true)
        .with_ignore_output_shapes(true)
        .model_for_read(&mut std::io::Cursor::new(bytes))?
        .with_input_fact(0, f32::fact([batch, WIN]).into())?;
    let outlets = m.output_outlets()?.to_vec();
    let mut keep = vec![];
    let wants: &[&str] = if spatial {
        &["embedding", "spatial_embedding", "label"]
    } else {
        &["embedding", "label"]
    };
    for &want in wants {
        match outlets.iter().find(|o| m.outlet_label(**o) == Some(want)) {
            Some(o) => keep.push(*o),
            None if want == "spatial_embedding" => {
                return Err(tract_onnx::tract_core::internal::format_err!(
                    "model has no spatial_embedding output"
                ));
            }
            None => {}
        }
    }
    if keep.is_empty() {
        let names: Vec<_> = outlets.iter().map(|o| m.outlet_label(*o)).collect();
        return Err(tract_onnx::tract_core::internal::format_err!(
            "no embedding or label output among {names:?}"
        ));
    }
    m.select_output_outlets(&keep)?;
    let typed = m.into_typed()?.into_decluttered()?;
    let size = |i: usize| -> TractResult<usize> {
        let fact = typed.outlet_fact(typed.output_outlets()?[i])?;
        Ok(fact
            .shape
            .as_concrete()
            .map_or(0, |s| s[1..].iter().product()))
    };
    let spatial = if spatial { size(1)? } else { 0 };
    let first_label = 1 + (spatial > 0) as usize;
    let classes = if keep.len() > first_label {
        size(first_label)?
    } else {
        0
    };
    let plan = typed.into_optimized()?.into_runnable()?;
    Ok(Model {
        plan,
        spatial,
        classes,
    })
}

impl Model {
    pub fn out_len(&self) -> usize {
        EMB + self.spatial + self.classes
    }
}

/// Run `batch` windows (batch * WIN samples). Returns, per window, EMB embedding
/// values, then the spatial embedding if kept, then `classes` logits.
pub fn run_windows(m: &Model, batch: usize, samples: &[f32]) -> TractResult<Vec<f32>> {
    let input = Tensor::from_shape(&[batch, WIN], samples)?;
    let out = m.plan.run(tvec!(input.into()))?;
    let views = out
        .iter()
        .map(|t| t.to_plain_array_view::<f32>())
        .collect::<TractResult<Vec<_>>>()?;
    let mut v = Vec::with_capacity(batch * m.out_len());
    for b in 0..batch {
        for o in &views {
            v.extend(o.index_axis(tract_ndarray::Axis(0), b).iter().copied());
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

        /// For the classifier in head.rs: `run` also returns the spatial embedding,
        /// between the embedding and any logits.
        pub fn with_spatial(model: &[u8]) -> Result<Perch, JsError> {
            load_model_with(model, 1, true)
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

    /// Perch's classifier, from export_head.py's int8 file.
    #[wasm_bindgen]
    pub struct PerchHead(head::Head);

    #[wasm_bindgen]
    impl PerchHead {
        #[wasm_bindgen(constructor)]
        pub fn new(file: Vec<u8>) -> Result<PerchHead, JsError> {
            head::Head::from_bytes(file)
                .map(PerchHead)
                .map_err(|e| JsError::new(&e))
        }

        pub fn classes(&self) -> usize {
            self.0.classes
        }

        /// The spatial embedding (64 x 1536 values) in; label logits out.
        pub fn scores(&self, spatial: &[f32]) -> Vec<f32> {
            self.0.scores(spatial)
        }
    }

    /// Bytes of WASM memory in use (it only grows).
    #[wasm_bindgen]
    pub fn wasm_memory_bytes() -> usize {
        core::arch::wasm32::memory_size(0) * 65536
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
