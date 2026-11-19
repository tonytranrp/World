//! Materials: the id stored in every voxel and the table that gives it meaning.
//!
//! The table is filled from `assets/config/materials.ron` by the Engine layer.
//! Parsing goes through [`MaterialTable::new`], so a table that exists is a
//! table that passed validation.

mod def;
mod id;
mod table;

pub use def::{MaterialDef, Rgb};
pub use id::MaterialId;
pub use table::{MaterialError, MaterialErrors, MaterialTable};
