//! BirdBox "Just-Bird" (a YOLO11s box detector for bird sounds on spectrogram images)
//! with the released network file in tract and everything around it in Rust: the
//! log-floor spectrogram image, NMS, and window fusion.
//!
//! Builds natively (the `native` binary) and for wasm32-unknown-unknown with
//! wasm-bindgen (the app's browser target), driven from node by `bench-bindgen.mjs`.

pub mod frontend;
pub mod inferno;
pub mod network;
pub mod post;

pub use frontend::{FileFrames, Frontend};
pub use network::Network;
pub use post::{Merged, RawBox};

/// wasm32-unknown-unknown (the app's browser target), through wasm-bindgen. The steps
/// are separate calls so node can time each one.
#[cfg(all(target_arch = "wasm32", target_os = "unknown"))]
mod bindgen {
    use super::*;
    use wasm_bindgen::prelude::*;

    #[wasm_bindgen]
    pub struct BirdBox {
        net: Network,
        fe: Frontend,
        ff: Option<FileFrames>,
        raw: Vec<RawBox>,
    }

    fn js(e: impl std::fmt::Display) -> JsError {
        JsError::new(&format!("{e:#}"))
    }

    #[wasm_bindgen]
    impl BirdBox {
        #[wasm_bindgen(constructor)]
        pub fn new(model: &[u8]) -> Result<BirdBox, JsError> {
            Ok(BirdBox { net: network::load(model).map_err(js)?, fe: Frontend::new(), ff: None, raw: vec![] })
        }

        /// Spectrogram frames of a whole file (32 kHz mono); returns the window count.
        pub fn frames(&mut self, audio: &[f32]) -> usize {
            let ff = frontend::file_frames(&self.fe, audio);
            let n = ff.n_clips;
            self.ff = Some(ff);
            self.raw.clear();
            n
        }

        /// Floor and image of window c.
        pub fn image(&self, c: usize) -> Vec<f32> {
            let ff = self.ff.as_ref().unwrap();
            let floor = frontend::rolling_floor(ff, c);
            frontend::clip_image(ff, c, &floor)
        }

        pub fn network(&self, image: &[f32]) -> Result<Vec<f32>, JsError> {
            network::run(&self.net, image).map_err(js)
        }

        /// NMS on window c's head; keeps the boxes for `fuse`. Returns how many.
        pub fn decode(&mut self, c: usize, head: &[f32], conf: f32) -> usize {
            let b = post::decode(head, FileFrames::clip_start_s(c), conf);
            let n = b.len();
            self.raw.extend(b);
            n
        }

        /// Window fusion of all decoded boxes: 7 values per box (t0, t1, f0, f1,
        /// avg_conf, max_conf, merged).
        pub fn fuse(&self, phrase_gap: f64) -> Vec<f64> {
            let ff = self.ff.as_ref().unwrap();
            let raw = post::limit_to_content(self.raw.clone(), ff.content_end_s(), ff.file_end_s());
            post::fuse(&raw, ff.file_end_s(), phrase_gap)
                .iter()
                .flat_map(|m| [m.t0, m.t1, m.f0, m.f1, m.avg_conf, m.max_conf, m.merged])
                .collect()
        }
    }

    /// Bytes of WASM memory in use (it only grows).
    #[wasm_bindgen]
    pub fn wasm_memory_bytes() -> usize {
        core::arch::wasm32::memory_size(0) * 65536
    }
}
