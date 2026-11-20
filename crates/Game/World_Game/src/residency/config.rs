//! Residency tuning (decision 0006): radii, budgets and threads.

use serde::{Deserialize, Serialize};
use world_core::ConfigAsset;

/// How much of the world is kept, where, and with how many threads.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ResidencyConfig {
    /// The world seed.
    pub seed: u64,
    /// Chunks whose centre is within this distance of the player are loaded
    /// at full detail, in metres.
    pub full_radius_m: f32,
    /// Full-detail chunks are evicted only beyond this distance, in metres.
    pub evict_radius_m: f32,
    /// Coarse chunk columns are kept out to this horizontal distance, in
    /// metres.
    pub coarse_radius_m: f32,
    /// Coarse cells are `4^level` voxels: 2 is a quarter metre.
    pub coarse_cell_level: u32,
    /// Full-detail chunks may hold at most this much RAM, in MiB.
    pub hot_cap_mib: u32,
    /// Compressed chunks kept after eviction (edited and generated), in MiB.
    pub warm_cap_mib: u32,
    /// Edited chunks in the warm cache age this many times more slowly than
    /// generated ones, so they stay longer (losing one costs a region file
    /// read on top of generation).
    pub warm_edited_weight: f32,
    /// Coarse chunks, in MiB.
    pub coarse_cap_mib: u32,
    /// Worker threads for generation and encoding.
    pub workers: u32,
    /// Chunks joined to the world per frame at most.
    pub grafts_per_frame: u32,
    /// When the edit log grows this much past its size after the last
    /// checkpoint, the edited chunks are written to their region files and
    /// the log restarts from a checkpoint, in KiB.
    pub log_checkpoint_kib: u32,
    /// Where region files and the edit log live, relative to the working
    /// directory.
    pub save_dir: String,
    /// The player's velocity is averaged over about this long, in seconds.
    pub velocity_smoothing_secs: f32,
    /// While the player moves, full-detail chunks near where they will be
    /// this many seconds ahead load first, and a lookahead sphere there is
    /// loaded too. 0 turns prefetching off.
    pub prefetch_lookahead_secs: f32,
    /// The lookahead sphere's radius, in metres. Its chunks load only once
    /// every chunk within the full-detail radius is requested, and only out
    /// to the evict radius.
    pub prefetch_radius_m: f32,
    /// Column footprints (the heightfield under a chunk, 8 MiB each) shared
    /// by the workers, so chunks stacked in a column compute it once, in
    /// MiB. 0 turns the cache off.
    pub column_cache_mib: u32,
    /// Chunks entirely below the terrain surface load as if they were this
    /// much farther away, in metres, so the visible ones come first.
    pub buried_delay_m: f32,
}

impl Default for ResidencyConfig {
    fn default() -> Self {
        Self {
            seed: 0x5EED,
            full_radius_m: 64.0,
            evict_radius_m: 96.0,
            coarse_radius_m: 384.0,
            coarse_cell_level: 2,
            hot_cap_mib: 1300,
            warm_cap_mib: 400,
            warm_edited_weight: 4.0,
            coarse_cap_mib: 500,
            workers: 4,
            grafts_per_frame: 8,
            log_checkpoint_kib: 16384,
            save_dir: "saves/world".into(),
            velocity_smoothing_secs: 0.5,
            prefetch_lookahead_secs: 2.0,
            prefetch_radius_m: 64.0,
            column_cache_mib: 256,
            buried_delay_m: 32.0,
        }
    }
}

impl ConfigAsset for ResidencyConfig {
    const NAME: &'static str = "residency";
    const PATH: &'static str = "config/residency.ron";

    fn validate(&self) -> Result<(), Vec<String>> {
        let mut problems = Vec::new();
        let mut range = |field: &str, value: f64, lo: f64, hi: f64| {
            if !(lo..=hi).contains(&value) || value.is_nan() {
                problems.push(format!("{field} = {value} is outside {lo}..={hi}"));
            }
        };
        range("full_radius_m", self.full_radius_m.into(), 16.0, 512.0);
        range("evict_radius_m", self.evict_radius_m.into(), 16.0, 1024.0);
        range("coarse_radius_m", self.coarse_radius_m.into(), 0.0, 8192.0);
        range("coarse_cell_level", self.coarse_cell_level.into(), 1.0, 4.0);
        range("hot_cap_mib", self.hot_cap_mib.into(), 64.0, 65536.0);
        range("warm_cap_mib", self.warm_cap_mib.into(), 0.0, 65536.0);
        range(
            "warm_edited_weight",
            self.warm_edited_weight.into(),
            1.0,
            64.0,
        );
        range("coarse_cap_mib", self.coarse_cap_mib.into(), 0.0, 65536.0);
        range("workers", self.workers.into(), 0.0, 64.0);
        range(
            "grafts_per_frame",
            self.grafts_per_frame.into(),
            1.0,
            1024.0,
        );
        range(
            "log_checkpoint_kib",
            self.log_checkpoint_kib.into(),
            64.0,
            1_048_576.0,
        );
        range(
            "velocity_smoothing_secs",
            self.velocity_smoothing_secs.into(),
            0.0,
            10.0,
        );
        range(
            "prefetch_lookahead_secs",
            self.prefetch_lookahead_secs.into(),
            0.0,
            10.0,
        );
        range(
            "prefetch_radius_m",
            self.prefetch_radius_m.into(),
            16.0,
            1024.0,
        );
        range(
            "column_cache_mib",
            self.column_cache_mib.into(),
            0.0,
            16384.0,
        );
        range("buried_delay_m", self.buried_delay_m.into(), 0.0, 1024.0);
        if self.evict_radius_m < self.full_radius_m {
            problems.push("evict_radius_m must be at least full_radius_m".into());
        }
        if self.save_dir.trim().is_empty() {
            problems.push("save_dir is empty".into());
        }
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
    fn the_default_is_valid_and_round_trips_through_ron() {
        let config = ResidencyConfig::default();
        assert_eq!(config.validate(), Ok(()));
        let text = ron::to_string(&config).unwrap();
        assert_eq!(ron::from_str::<ResidencyConfig>(&text).unwrap(), config);
    }

    #[test]
    fn every_problem_is_listed() {
        let bad = ResidencyConfig {
            full_radius_m: 200.0,
            evict_radius_m: 100.0,
            workers: 100,
            log_checkpoint_kib: 1,
            save_dir: " ".into(),
            velocity_smoothing_secs: f32::NAN,
            prefetch_lookahead_secs: -1.0,
            prefetch_radius_m: 4.0,
            warm_edited_weight: 0.5,
            column_cache_mib: 20000,
            buried_delay_m: f32::NAN,
            ..ResidencyConfig::default()
        };
        assert_eq!(bad.validate().unwrap_err().len(), 10);
    }

    #[test]
    fn the_shipped_file_matches_the_default() {
        let path = concat!(
            env!("CARGO_MANIFEST_DIR"),
            "/../../../assets/config/residency.ron"
        );
        let text = std::fs::read_to_string(path).unwrap();
        let shipped: ResidencyConfig = ron::from_str(&text).unwrap();
        assert_eq!(shipped, ResidencyConfig::default());
    }
}
