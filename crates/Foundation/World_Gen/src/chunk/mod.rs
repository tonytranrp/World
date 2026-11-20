//! Chunk orchestration: runs the stages for a chunk's footprint, at full
//! resolution or on a coarse lattice.
//!
//! The order is continental (the macro lattice), regional (the detail lattice
//! and slopes), then surface (one column at a time). Lattice grids are a
//! cache: every value in them is a pure function of its lattice index, so a
//! column comes out identical from a whole chunk, a coarse pass or a single
//! column query, and neighbouring chunks agree at their borders because they
//! evaluate the same global lattices. The margins a footprint needs are
//! computed from the lattice spacings (`regional::field`), not guessed.

pub mod coarse_source;
pub mod columns;
pub mod coord;
pub mod source;

pub use coarse_source::CoarseSource;
pub use columns::{ChunkColumns, CoarseColumns, MAX_COARSE_LEVEL};
pub use coord::{CHUNK_SHIFT, CHUNK_SIZE, CHUNKS_PER_AXIS, ChunkCoord, ChunkOutOfWorld};
pub use source::ChunkSource;

use crate::GeneratorId;
use crate::continental::Continental;
use crate::regional::field::{ColumnInputs, RowInputs, column_inputs};
use crate::regional::{ColumnRect, DetailGrid, MacroGrid, Regional};
use crate::surface::{Column, Surface};

/// The prepared stages of one generator.
#[derive(Clone)]
pub(crate) struct Stages {
    pub continental: Continental,
    pub regional: Regional,
    pub surface: Surface,
}

impl Stages {
    /// The footprint rectangle of a chunk.
    fn chunk_rect(chunk: ChunkCoord) -> ColumnRect {
        let min = chunk.min_voxel();
        ColumnRect {
            x0: min.x,
            z0: min.z,
            nx: CHUNK_SIZE,
            nz: CHUNK_SIZE,
        }
    }

    /// One world column, computed from grids covering only it.
    pub fn column(&self, x: i32, z: i32) -> Column {
        let rect = ColumnRect::single(x, z);
        let macros = MacroGrid::covering(&self.continental, &rect);
        self.column_with(&macros, x, z)
    }

    /// One world column, with a macro grid that covers it.
    fn column_with(&self, macros: &MacroGrid, x: i32, z: i32) -> Column {
        let rect = ColumnRect::single(x, z);
        let detail = DetailGrid::covering(&self.regional, macros, &rect);
        self.surface
            .column(x, z, &column_inputs(macros, &detail, x, z))
    }

    /// Every column of a chunk's footprint.
    pub fn columns(&self, generator: GeneratorId, chunk: ChunkCoord) -> ChunkColumns {
        let rect = Self::chunk_rect(chunk);
        let macros = MacroGrid::covering(&self.continental, &rect);
        let detail = DetailGrid::covering(&self.regional, &macros, &rect);
        let n = CHUNK_SIZE as i32;
        let mut columns = Vec::with_capacity((CHUNK_SIZE * CHUNK_SIZE) as usize);
        let (mut lo, mut hi) = (i32::MAX, i32::MIN);
        // A row of columns at a time: equal to `column_inputs` per column.
        let mut rows = RowInputs::new(&macros, &detail, rect.x0, CHUNK_SIZE);
        let mut inputs = vec![ColumnInputs::default(); CHUNK_SIZE as usize];
        for z in rect.z0..rect.z0 + n {
            rows.row(z, &mut inputs);
            for (x, inputs) in (rect.x0..).zip(&inputs) {
                let column = self.surface.column(x, z, inputs);
                lo = lo.min(column.height);
                hi = hi.max(column.height);
                columns.push(column);
            }
        }
        ChunkColumns {
            generator,
            chunk,
            layers: self.surface.layers(),
            columns,
            height_range: (lo, hi),
        }
    }

    /// The footprint sampled every `4^level` voxels; `level` is
    /// `1..=MAX_COARSE_LEVEL` (checked by the caller).
    ///
    /// Level 1 samples every detail point, so it computes the chunk's detail
    /// grid once. Coarser levels need a sixteenth or less of it, so each
    /// sample computes its own 4 x 4 detail patch from a shared macro grid.
    /// Both give exactly the columns of [`columns`](Self::columns).
    pub fn coarse(&self, generator: GeneratorId, chunk: ChunkCoord, level: u32) -> CoarseColumns {
        let rect = Self::chunk_rect(chunk);
        let spacing = 1i32 << (2 * level);
        let n = (CHUNK_SIZE >> (2 * level)) as i32;
        let macros = MacroGrid::covering(&self.continental, &rect);
        let detail = (level == 1).then(|| DetailGrid::covering(&self.regional, &macros, &rect));
        let mut columns = Vec::with_capacity((n * n) as usize);
        for j in 0..n {
            for i in 0..n {
                let (x, z) = (rect.x0 + i * spacing, rect.z0 + j * spacing);
                let column = match &detail {
                    Some(detail) => {
                        self.surface
                            .column(x, z, &column_inputs(&macros, detail, x, z))
                    }
                    None => self.column_with(&macros, x, z),
                };
                columns.push(column);
            }
        }
        CoarseColumns {
            generator,
            chunk,
            level,
            layers: self.surface.layers(),
            columns,
        }
    }
}
