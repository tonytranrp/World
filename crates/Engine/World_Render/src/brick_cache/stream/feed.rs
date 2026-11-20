//! The input contract: what the main world hands the renderer.

use std::sync::Arc;

use bevy::prelude::Resource;
use world_core::{BrickPos, DirtyBricks, TreeSnapshot};

/// The world the renderer mirrors. Whoever owns the `VoxelTree` updates it
/// once per frame after applying that frame's edits:
///
/// ```ignore
/// let dirty = tree.apply(&edits)?;
/// feed.publish(Arc::new(tree.snapshot()), &dirty);
/// ```
///
/// Changed bricks accumulate until the render world takes them, so each
/// change is uploaded exactly once even if several main-world frames pass
/// between extractions. A different world (a load, a new seed) goes through
/// [`replace`](Self::replace), which makes the cache start over.
#[derive(Resource, Default)]
pub struct VoxelWorldFeed {
    snapshot: Option<Arc<TreeSnapshot>>,
    dirty: Vec<BrickPos>,
    refresh: bool,
    /// Counts [`replace`](Self::replace) and [`clear`](Self::clear) calls:
    /// a publisher that remembers it can tell when another one took over.
    generation: u64,
}

/// What the render world takes from the feed in one extraction.
#[derive(Clone, Default)]
pub struct FeedFrame {
    pub snapshot: Option<Arc<TreeSnapshot>>,
    pub dirty: Vec<BrickPos>,
    pub refresh: bool,
}

impl VoxelWorldFeed {
    /// The next state of the same world, and the bricks that changed since
    /// the previous publish.
    pub fn publish(&mut self, snapshot: Arc<TreeSnapshot>, dirty: &DirtyBricks) {
        self.snapshot = Some(snapshot);
        self.dirty.extend_from_slice(dirty.as_slice());
    }

    /// A different world: everything resident is dropped and rebuilt.
    pub fn replace(&mut self, snapshot: Arc<TreeSnapshot>) {
        self.snapshot = Some(snapshot);
        self.dirty.clear();
        self.refresh = true;
        self.generation += 1;
    }

    /// Stops rendering any world.
    pub fn clear(&mut self) {
        self.snapshot = None;
        self.dirty.clear();
        self.refresh = true;
        self.generation += 1;
    }

    /// Which world the feed holds: it changes on every
    /// [`replace`](Self::replace) and [`clear`](Self::clear), never on a
    /// [`publish`](Self::publish).
    pub fn generation(&self) -> u64 {
        self.generation
    }

    pub fn snapshot(&self) -> Option<&Arc<TreeSnapshot>> {
        self.snapshot.as_ref()
    }

    /// Hands the accumulated changes to the render world.
    pub fn take(&mut self) -> FeedFrame {
        FeedFrame {
            snapshot: self.snapshot.clone(),
            dirty: std::mem::take(&mut self.dirty),
            refresh: std::mem::take(&mut self.refresh),
        }
    }
}

impl FeedFrame {
    /// Folds a later extraction into this one, for frames the render world
    /// could not process yet.
    pub fn merge(&mut self, later: FeedFrame) {
        if later.refresh {
            *self = later;
            return;
        }
        self.snapshot = later.snapshot;
        self.dirty.extend(later.dirty);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use world_core::{Edit, MaterialId, VoxelPos, VoxelTree};

    #[test]
    fn generation_changes_on_replace_and_clear_only() {
        let tree = VoxelTree::new();
        let mut feed = VoxelWorldFeed::default();
        let g0 = feed.generation();
        feed.replace(Arc::new(tree.snapshot()));
        let g1 = feed.generation();
        assert_ne!(g0, g1);
        feed.publish(Arc::new(tree.snapshot()), &DirtyBricks::default());
        let _ = feed.take();
        assert_eq!(feed.generation(), g1, "publish and take keep the world");
        feed.clear();
        assert_ne!(feed.generation(), g1);
    }

    #[test]
    fn changes_are_taken_exactly_once() {
        let mut tree = VoxelTree::new();
        let mut feed = VoxelWorldFeed::default();
        let dirty = tree
            .apply(&[Edit::new(VoxelPos::new(1, 2, 3), MaterialId(1))])
            .unwrap();
        feed.publish(Arc::new(tree.snapshot()), &dirty);
        let first = feed.take();
        assert_eq!(first.dirty, dirty.as_slice());
        assert!(first.snapshot.is_some());
        assert!(feed.take().dirty.is_empty());
    }

    #[test]
    fn a_replacement_wins_over_pending_changes() {
        let tree = VoxelTree::new();
        let mut pending = FeedFrame {
            dirty: vec![BrickPos::new(0, 0, 0)],
            ..Default::default()
        };
        let mut feed = VoxelWorldFeed::default();
        feed.replace(Arc::new(tree.snapshot()));
        pending.merge(feed.take());
        assert!(pending.refresh && pending.dirty.is_empty());
    }
}
