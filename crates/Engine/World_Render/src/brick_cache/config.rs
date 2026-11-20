//! Brick cache tuning, loaded from `assets/config/brick_cache.ron`.
//!
//! Pool sizes are read once when the render device starts (pools never grow);
//! everything else is read every frame and reloads live.

use serde::{Deserialize, Serialize};
use world_core::ConfigAsset;

use crate::tier::Profile;

const MIB: u64 = 1024 * 1024;

/// Which stage of decision 0007 drives residency.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Stage {
    /// Stage 1: the CPU chooses nodes and bricks by distance, with hysteresis.
    #[default]
    Distance,
    /// Stage 2: bricks are paged from ray requests; nodes stay distance-driven
    /// and are also expanded on request.
    RayGuided,
}

/// Pool sizes and the per-frame upload budget of one machine class.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PoolSizes {
    /// Node pool, MiB. `1..=4096`.
    pub node_pool_mib: u32,
    /// Brick pool, MiB (the sum of its segments). `16..=16384`.
    pub brick_pool_mib: u32,
    /// Upload budget per frame, MiB. `1..=256`, at most the brick pool.
    pub upload_budget_mib: u32,
}

impl PoolSizes {
    pub fn node_pool_bytes(&self) -> u64 {
        u64::from(self.node_pool_mib) * MIB
    }

    pub fn brick_pool_bytes(&self) -> u64 {
        u64::from(self.brick_pool_mib) * MIB
    }

    pub fn upload_budget_bytes(&self) -> u64 {
        u64::from(self.upload_budget_mib) * MIB
    }
}

/// Level of detail: when descending further is worth it.
#[derive(Clone, Copy, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LodTuning {
    /// Stage 1 refines a node when the cells it would reveal project to at
    /// least this many pixels. `0.05..=64.0`.
    pub detail_pixels: f32,
    /// Stage 1 keeps a refined node until its cells shrink below
    /// `detail_pixels * hysteresis`. `0.1..=1.0`.
    pub hysteresis: f32,
    /// Rays stop descending where a cell is smaller than this many pixel
    /// footprints; `0.0` traces at full resolution. `0.0..=16.0`.
    pub footprint_bias: f32,
    /// Records stage 1 examines per frame; a walk over more resumes next
    /// frame. `1024..=16777216`.
    pub max_visits_per_frame: u32,
    /// Stage 1 looks for nodes to refine again once the camera moved this
    /// many voxels, or the cache changed; `0` looks every frame.
    /// `0.0..=4096.0`.
    pub regrow_voxels: f32,
    /// A stage 1 pass in progress starts over from the root only once the
    /// camera moved this many voxels since it started (at least
    /// `regrow_voxels`); smaller moves let it finish, since every node is
    /// checked against the current view when it is taken. `0.0..=65536.0`.
    pub restart_voxels: f32,
    /// Nodes stage 1 or 2 expands per frame at most; the rest wait for the
    /// next frame. `1..=1048576`.
    pub max_expansions_per_frame: u32,
}

/// Everything the brick cache reads from its config file.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct BrickCacheConfig {
    pub stage: Stage,
    /// Pools on the floor Mac (`--mac-floor`). Read at startup.
    pub floor: PoolSizes,
    /// Pools on the PC. Read at startup.
    pub pc: PoolSizes,
    /// Largest brick segment, MiB; `0` takes the adapter's binding limit.
    /// Read at startup. `0..=4096`.
    pub max_segment_mib: u32,
    /// Expected mean brick block size, bytes: sizes the brick id table and
    /// the allocator's bookkeeping. Read at startup. `64..=16384`.
    pub mean_brick_bytes: u32,
    /// Evict when the brick pool is fuller than this fraction. `0.1..=1.0`.
    pub evict_threshold: f32,
    /// Fraction of resident bricks one eviction pass tries to free. `0.001..=0.5`.
    pub evict_fraction: f32,
    /// Bricks used within this many frames are never evicted. `2..=10000`.
    pub min_evict_age_frames: u32,
    /// Ray requests one frame can record. Read at startup. `64..=1048576`.
    pub request_capacity: u32,
    /// Victims one eviction pass can return. Read at startup. `64..=1048576`.
    pub victim_capacity: u32,
    pub lod: LodTuning,
    /// Node summaries (each over up to 64 children) that applying changed
    /// bricks and replaced chunks may compute per frame; further changes
    /// wait for later frames, nearest first. A replaced 16 m chunk costs
    /// one per mixed node in it. `1..=16777216`.
    pub max_change_summaries_per_frame: u32,
    /// Show a small built-in scene while nothing feeds the renderer a world.
    pub debug_scene: bool,
    /// Animate a carve in the debug scene, to exercise edit uploads.
    pub debug_scene_edits: bool,
}

impl Default for BrickCacheConfig {
    fn default() -> Self {
        Self {
            stage: Stage::Distance,
            floor: PoolSizes {
                node_pool_mib: 64,
                brick_pool_mib: 768,
                upload_budget_mib: 6,
            },
            pc: PoolSizes {
                node_pool_mib: 128,
                brick_pool_mib: 2048,
                upload_budget_mib: 24,
            },
            max_segment_mib: 0,
            mean_brick_bytes: 1024,
            evict_threshold: 0.9,
            evict_fraction: 0.05,
            min_evict_age_frames: 30,
            request_capacity: 16384,
            victim_capacity: 16384,
            lod: LodTuning {
                detail_pixels: 1.0,
                hysteresis: 0.7,
                footprint_bias: 1.0,
                max_visits_per_frame: 16384,
                regrow_voxels: 16.0,
                restart_voxels: 128.0,
                max_expansions_per_frame: 128,
            },
            max_change_summaries_per_frame: 2048,
            debug_scene: true,
            debug_scene_edits: true,
        }
    }
}

impl BrickCacheConfig {
    /// The pool sizes for a profile.
    pub fn sizes(&self, profile: Profile) -> PoolSizes {
        match profile {
            Profile::MacFloor => self.floor,
            Profile::Full => self.pc,
        }
    }
}

impl ConfigAsset for BrickCacheConfig {
    const NAME: &'static str = "brick_cache";
    const PATH: &'static str = "config/brick_cache.ron";

    fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut range = |field: &str, value: f64, lo: f64, hi: f64| {
            if !(lo..=hi).contains(&value) {
                problems.push(format!("{field} = {value} is outside {lo}..={hi}"));
            }
        };
        for (name, s) in [("floor", &self.floor), ("pc", &self.pc)] {
            range(
                &format!("{name}.node_pool_mib"),
                s.node_pool_mib.into(),
                1.0,
                4096.0,
            );
            range(
                &format!("{name}.brick_pool_mib"),
                s.brick_pool_mib.into(),
                16.0,
                16384.0,
            );
            range(
                &format!("{name}.upload_budget_mib"),
                s.upload_budget_mib.into(),
                1.0,
                f64::from(s.brick_pool_mib).min(256.0),
            );
        }
        range("max_segment_mib", self.max_segment_mib.into(), 0.0, 4096.0);
        range(
            "mean_brick_bytes",
            self.mean_brick_bytes.into(),
            64.0,
            16384.0,
        );
        range("evict_threshold", self.evict_threshold.into(), 0.1, 1.0);
        range("evict_fraction", self.evict_fraction.into(), 0.001, 0.5);
        range(
            "min_evict_age_frames",
            self.min_evict_age_frames.into(),
            2.0,
            10000.0,
        );
        range(
            "request_capacity",
            self.request_capacity.into(),
            64.0,
            1048576.0,
        );
        range(
            "victim_capacity",
            self.victim_capacity.into(),
            64.0,
            1048576.0,
        );
        range(
            "lod.detail_pixels",
            self.lod.detail_pixels.into(),
            0.05,
            64.0,
        );
        range("lod.hysteresis", self.lod.hysteresis.into(), 0.1, 1.0);
        range(
            "lod.footprint_bias",
            self.lod.footprint_bias.into(),
            0.0,
            16.0,
        );
        range(
            "lod.regrow_voxels",
            self.lod.regrow_voxels.into(),
            0.0,
            4096.0,
        );
        range(
            "max_change_summaries_per_frame",
            self.max_change_summaries_per_frame.into(),
            1.0,
            16777216.0,
        );
        range(
            "lod.restart_voxels",
            self.lod.restart_voxels.into(),
            0.0,
            65536.0,
        );
        range(
            "lod.max_expansions_per_frame",
            self.lod.max_expansions_per_frame.into(),
            1.0,
            1048576.0,
        );
        range(
            "lod.max_visits_per_frame",
            self.lod.max_visits_per_frame.into(),
            1024.0,
            16777216.0,
        );
        if problems.is_empty() {
            Ok(())
        } else {
            Err(problems)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_are_valid_and_follow_decision_0007() {
        let c = BrickCacheConfig::default();
        assert_eq!(c.validate(), Ok(()));
        let floor = c.sizes(Profile::MacFloor);
        assert_eq!((floor.node_pool_mib, floor.brick_pool_mib), (64, 768));
        assert!((4..=8).contains(&floor.upload_budget_mib));
        let pc = c.sizes(Profile::Full);
        assert_eq!((pc.node_pool_mib, pc.brick_pool_mib), (128, 2048));
        assert!((16..=32).contains(&pc.upload_budget_mib));
    }

    #[test]
    fn every_bad_field_is_reported() {
        let mut c = BrickCacheConfig::default();
        c.floor.upload_budget_mib = 0;
        c.evict_threshold = f32::NAN;
        c.lod.hysteresis = 2.0;
        let problems = c.validate().unwrap_err();
        assert_eq!(problems.len(), 3, "{problems:?}");
    }
}
