//! CPU-side streaming: what is resident, what changes this frame, and the
//! upload that carries it.
//!
//! - `feed`: the input contract, the snapshot and changed bricks from the main world
//! - `key`: names of nodes in the mirrored tree
//! - `residency`: the residency table and every operation on it
//! - `select`: stage 1, distance-driven selection with hysteresis
//! - `frame`: one frame's work under the byte budget, edited bricks first
//! - `batch`: the upload buffer the apply pass reads, and a CPU model of the pass
//! - `upload`: recording the upload and the apply pass on the GPU

pub mod batch;
pub mod feed;
pub mod frame;
pub mod key;
pub mod residency;
pub mod select;
pub mod upload;
