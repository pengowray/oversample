//! BirdBox's Just-Bird box detector (BirdBox v2.1.0): a box round each bird
//! sound, made from the same 32 kHz audio BirdNET+ V3.0 hears.
//!
//! - [`frontend`]: the window images, from a stream of audio.
//! - [`network`]: the network in tract, cut out of BirdBox's demo graph.
//! - NMS and the window fusion are in `oversample_ipc::classifier::boxes`,
//!   so the app can merge the boxes again with other settings.

pub mod frontend;
mod inferno;
pub mod network;

pub use frontend::BoxStream;
pub use network::Network;
use oversample_ipc::classifier::boxes::{decode, RawBox, CLIP_HOP_S, KEEP_CONF};

/// The boxes of each window the audio completes: `(window, boxes)`.
pub type WindowBoxes = Vec<(u32, Vec<RawBox>)>;

/// Run the network on each window `clips` gives and keep its boxes at
/// [`KEEP_CONF`] or above.
pub fn boxes_of(
    net: &Network,
    clips: Vec<frontend::Clip>,
) -> tract_onnx::prelude::TractResult<WindowBoxes> {
    clips
        .into_iter()
        .map(|c| {
            let head = net.run(&c.image)?;
            Ok((
                c.index as u32,
                decode(&head, c.index as f64 * CLIP_HOP_S, KEEP_CONF),
            ))
        })
        .collect()
}
