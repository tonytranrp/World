//! Deterministic scenes shared by the World_Core benchmarks.
//!
//! Every input comes from a fixed seed and integer arithmetic only, so a scene
//! is identical on every run and every machine (no wall clock, no floating
//! point). Each bench binary uses a subset, hence the `dead_code` allowance.

#![allow(dead_code)]

use world_core::{Edit, MaterialId, RegionSource, VoxelBox, VoxelPos, VoxelTree};

/// A tiny deterministic generator (Knuth's MMIX LCG, upper bits).
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u32 {
        self.0 = self
            .0
            .wrapping_mul(6364136223846793005)
            .wrapping_add(1442695040888963407);
        (self.0 >> 33) as u32
    }

    /// Uniform in `0..n`.
    pub fn below(&mut self, n: u32) -> u32 {
        self.next() % n
    }

    /// A position in the cube `[min, min + edge)` on every axis.
    pub fn pos_in(&mut self, min: VoxelPos, edge: u32) -> VoxelPos {
        VoxelPos::new(
            min.x + self.below(edge) as i32,
            min.y + self.below(edge) as i32,
            min.z + self.below(edge) as i32,
        )
    }

    /// A solid material from `1..=count`.
    pub fn material(&mut self, count: u16) -> MaterialId {
        MaterialId(1 + self.below(u32::from(count)) as u16)
    }
}

/// `count` edits at random positions of the cube at `min` with `edge` voxels
/// per axis, each a random solid material from `1..=materials`.
pub fn random_edits(
    count: usize,
    min: VoxelPos,
    edge: u32,
    materials: u16,
    seed: u64,
) -> Vec<Edit> {
    let mut rng = Lcg(seed);
    (0..count)
        .map(|_| Edit::new(rng.pos_in(min, edge), rng.material(materials)))
        .collect()
}

/// Every voxel of the cube at `min` with `edge` voxels per axis set to a
/// random solid material from `1..=materials`: the densest content a tree
/// holds, every leaf with a full mask and a multi-entry palette.
pub fn dense_block(min: VoxelPos, edge: u32, materials: u16, seed: u64) -> VoxelTree {
    let mut rng = Lcg(seed);
    let mut tree = VoxelTree::new();
    let edge = edge as i32;
    for z in 0..edge {
        for y in 0..edge {
            for x in 0..edge {
                let pos = VoxelPos::new(min.x + x, min.y + y, min.z + z);
                tree.set(pos, rng.material(materials)).unwrap();
            }
        }
    }
    tree
}

/// One solid material filling the cube at `min` with `edge` voxels per axis.
/// Aligned cubes become a handful of uniform children, so this is cheap.
pub fn solid_block(min: VoxelPos, edge: u32, material: MaterialId) -> VoxelTree {
    let mut tree = VoxelTree::new();
    tree.fill_box(VoxelBox::from_min_size(min, [edge; 3]), material)
        .unwrap();
    tree
}

/// A large tree for copy-on-write measurements: the cube at the origin with
/// `edge` voxels per axis, in z-slabs one voxel thick that alternate between
/// two materials. Every leaf is mixed (two-entry palette, 1-bit indices), so
/// the storage is large, yet it builds with `edge` box fills instead of
/// `edge^3` single writes.
pub fn striped_block(edge: u32) -> VoxelTree {
    let mut tree = VoxelTree::new();
    tree.fill_box(
        VoxelBox::from_min_size(VoxelPos::ORIGIN, [edge; 3]),
        MaterialId(1),
    )
    .unwrap();
    for z in (1..edge as i32).step_by(2) {
        tree.fill_box(
            VoxelBox::from_min_size(VoxelPos::new(0, 0, z), [edge, edge, 1]),
            MaterialId(2),
        )
        .unwrap();
    }
    tree
}

/// Material ids used by the terrain.
pub const STONE: MaterialId = MaterialId(1);
pub const DIRT: MaterialId = MaterialId(2);
pub const GRASS: MaterialId = MaterialId(3);

/// The terrain's lowest voxel. A multiple of 256, so everything below the
/// surface packs into large uniform children, as deep ground would.
pub const TERRAIN_FLOOR: i32 = -256;

/// Rolling terrain over `[0, width)` in x and z: a heightmap of integer value
/// noise (octaves of 64, 16 and 4 voxels), heights about 0 to 70 voxels
/// (roughly 1 m), stone below three voxels of dirt and one of grass.
pub struct Terrain {
    pub width: u32,
    /// Column heights, `heights[x + z * width]`: the y of the top voxel.
    pub heights: Vec<i32>,
}

impl Terrain {
    pub fn new(width: u32, seed: u64) -> Self {
        let mut heights = Vec::with_capacity((width * width) as usize);
        for z in 0..width as i32 {
            for x in 0..width as i32 {
                let h = value_noise(seed, x, z, 64, 48)
                    + value_noise(seed ^ 0x9e37, x, z, 16, 16)
                    + value_noise(seed ^ 0x7f4a, x, z, 4, 6);
                heights.push(h);
            }
        }
        Self { width, heights }
    }

    pub fn height(&self, x: i32, z: i32) -> i32 {
        self.heights[(x + z * self.width as i32) as usize]
    }

    /// Builds the terrain with three box fills per column.
    pub fn build(&self) -> VoxelTree {
        let mut tree = VoxelTree::new();
        for z in 0..self.width as i32 {
            for x in 0..self.width as i32 {
                let h = self.height(x, z);
                let column = |from: i32, to: i32| {
                    VoxelBox::new(VoxelPos::new(x, from, z), VoxelPos::new(x + 1, to, z + 1))
                };
                tree.fill_box(column(TERRAIN_FLOOR, h - 3), STONE).unwrap();
                tree.fill_box(column(h - 3, h), DIRT).unwrap();
                tree.fill_box(column(h, h + 1), GRASS).unwrap();
            }
        }
        tree
    }

    /// Builds the same terrain bottom-up through a [`RegionSource`], as
    /// World_Gen's chunks are built (decision 0006): same content, the
    /// builder's storage layout and no free words.
    pub fn build_bottom_up(&self) -> VoxelTree {
        VoxelTree::from_source(&TerrainSource(self))
    }

    /// The material of voxel `(x, y, z)`: what [`build`](Self::build) writes.
    pub fn material(&self, x: i32, y: i32, z: i32) -> MaterialId {
        let w = self.width as i32;
        if !(0..w).contains(&x) || !(0..w).contains(&z) || y < TERRAIN_FLOOR {
            return MaterialId::AIR;
        }
        let h = self.height(x, z);
        if y < h - 3 {
            STONE
        } else if y < h {
            DIRT
        } else if y == h {
            GRASS
        } else {
            MaterialId::AIR
        }
    }

    /// Solid voxels with at least one air neighbour among the four sides and
    /// the top, ignoring the outer edge and the flat floor: what a player
    /// sees and digs into.
    pub fn surface_voxels(&self) -> u64 {
        let w = self.width as i32;
        let mut total = 0;
        for z in 0..w {
            for x in 0..w {
                let h = self.height(x, z);
                let lowest_side = [(1, 0), (-1, 0), (0, 1), (0, -1)]
                    .into_iter()
                    .map(|(dx, dz)| (x + dx, z + dz))
                    .filter(|&(nx, nz)| (0..w).contains(&nx) && (0..w).contains(&nz))
                    .map(|(nx, nz)| self.height(nx, nz))
                    .min()
                    .unwrap_or(h);
                // The top voxel, plus every voxel a lower neighbour exposes.
                total += 1 + (h - 1 - lowest_side).max(0) as u64;
            }
        }
        total
    }

    /// A position just below the surface of a random column.
    pub fn near_surface(&self, rng: &mut Lcg) -> VoxelPos {
        let x = rng.below(self.width) as i32;
        let z = rng.below(self.width) as i32;
        let y = self.height(x, z) - rng.below(8) as i32;
        VoxelPos::new(x, y, z)
    }
}

/// A [`Terrain`] as a region source.
struct TerrainSource<'a>(&'a Terrain);

impl RegionSource for TerrainSource<'_> {
    fn uniform(&self, min: VoxelPos, size: u32) -> Option<MaterialId> {
        let t = self.0;
        let w = i64::from(t.width);
        let (lo, hi) = (i64::from(min.y), i64::from(min.y) + i64::from(size));
        let [x0, z0] = [min.x, min.z].map(i64::from);
        let (x1, z1) = (x0 + i64::from(size), z0 + i64::from(size));
        let (cx0, cx1, cz0, cz1) = (x0.max(0), x1.min(w), z0.max(0), z1.min(w));
        if cx0 >= cx1 || cz0 >= cz1 || hi <= i64::from(TERRAIN_FLOOR) {
            return Some(MaterialId::AIR);
        }
        let (mut low, mut high) = (i64::MAX, i64::MIN);
        for z in cz0..cz1 {
            for x in cx0..cx1 {
                let h = i64::from(t.height(x as i32, z as i32));
                low = low.min(h);
                high = high.max(h);
            }
        }
        if lo > high {
            return Some(MaterialId::AIR);
        }
        let inside = cx0 == x0 && cx1 == x1 && cz0 == z0 && cz1 == z1;
        (inside && lo >= i64::from(TERRAIN_FLOOR) && hi <= low - 3).then_some(STONE)
    }

    fn leaf(&self, min: VoxelPos, voxels: &mut [MaterialId; 64]) {
        for (slot, voxel) in (0..).zip(voxels.iter_mut()) {
            *voxel = self
                .0
                .material(min.x + slot % 4, min.y + slot / 4 % 4, min.z + slot / 16);
        }
    }
}

/// Bilinearly interpolated lattice noise in `0..amplitude`, integer only.
fn value_noise(seed: u64, x: i32, z: i32, cell: i32, amplitude: i32) -> i32 {
    let lattice = |cx: i32, cz: i32| -> i64 {
        let mut h = seed ^ ((cx as u32 as u64) << 32) ^ (cz as u32 as u64);
        h = h.wrapping_mul(0xff51afd7ed558ccd);
        h ^= h >> 33;
        h = h.wrapping_mul(0xc4ceb9fe1a85ec53);
        h ^= h >> 33;
        (h % amplitude as u64) as i64
    };
    let (cx, cz) = (x.div_euclid(cell), z.div_euclid(cell));
    let (fx, fz) = (x.rem_euclid(cell) as i64, z.rem_euclid(cell) as i64);
    let c = cell as i64;
    let top = lattice(cx, cz) * (c - fx) + lattice(cx + 1, cz) * fx;
    let bottom = lattice(cx, cz + 1) * (c - fx) + lattice(cx + 1, cz + 1) * fx;
    ((top * (c - fz) + bottom * fz) / (c * c)) as i32
}

/// A long session of play-like edits on a terrain: single voxels placed and
/// broken near the surface, small craters carved and small blocks built.
pub fn churn(tree: &mut VoxelTree, terrain: &Terrain, steps: usize, seed: u64) {
    let mut rng = Lcg(seed);
    for _ in 0..steps {
        let at = terrain.near_surface(&mut rng);
        match rng.below(10) {
            0..=5 => {
                let material = if rng.below(2) == 0 {
                    MaterialId::AIR
                } else {
                    rng.material(8)
                };
                tree.set(at, material).unwrap();
            }
            6..=8 => {
                let size = [0; 3].map(|_| 1 + rng.below(8));
                tree.fill_box(VoxelBox::from_min_size(at, size), MaterialId::AIR)
                    .unwrap();
            }
            _ => {
                let size = [0; 3].map(|_| 1 + rng.below(6));
                tree.fill_box(VoxelBox::from_min_size(at, size), rng.material(8))
                    .unwrap();
            }
        }
    }
}

/// Every voxel of a sphere of `radius` around `centre`.
pub fn sphere(centre: VoxelPos, radius: i32) -> Vec<VoxelPos> {
    let mut voxels = Vec::new();
    for z in -radius..=radius {
        for y in -radius..=radius {
            for x in -radius..=radius {
                if x * x + y * y + z * z <= radius * radius {
                    voxels.push(VoxelPos::new(centre.x + x, centre.y + y, centre.z + z));
                }
            }
        }
    }
    voxels
}
