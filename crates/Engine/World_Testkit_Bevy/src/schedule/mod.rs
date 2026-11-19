//! System-order randomization.
//!
//! Bevy can shuffle the order of systems inside a schedule, while still honouring
//! every explicit ordering constraint. A test that only passes in the default
//! order has a hidden ordering assumption; running it across several seeds
//! finds it.

use std::panic::{AssertUnwindSafe, catch_unwind, resume_unwind};

use bevy::app::{
    App, First, FixedFirst, FixedLast, FixedPostUpdate, FixedPreUpdate, FixedUpdate, Last,
    PostStartup, PostUpdate, PreStartup, PreUpdate, Startup, Update,
};
use bevy::ecs::schedule::{ScheduleLabel, SingleThreadedExecutor};
use world_testkit::profile::Profile;

fn shuffle<L: ScheduleLabel>(app: &mut App, label: L, seed: u64) {
    app.edit_schedule(label, |schedule| {
        let mut settings = schedule.get_build_settings();
        settings.shuffle_seed = Some(seed);
        schedule.set_build_settings(settings);
        // The multi-threaded executor can run systems in orders the shuffled
        // graph does not allow, which would make seeds irreproducible.
        schedule.set_executor(SingleThreadedExecutor::default());
        // A fresh executor forgets to apply queued commands when the schedule
        // ends, which would silently drop every spawn made in `Startup`.
        schedule.set_apply_final_deferred(true);
    });
}

/// Shuffles system order in every main schedule. Call before the first update,
/// because the order is fixed when a schedule is built.
pub fn randomize_schedules(app: &mut App, seed: u64) {
    shuffle(app, PreStartup, seed);
    shuffle(app, Startup, seed);
    shuffle(app, PostStartup, seed);
    shuffle(app, First, seed);
    shuffle(app, PreUpdate, seed);
    shuffle(app, Update, seed);
    shuffle(app, PostUpdate, seed);
    shuffle(app, Last, seed);
    shuffle(app, FixedFirst, seed);
    shuffle(app, FixedPreUpdate, seed);
    shuffle(app, FixedUpdate, seed);
    shuffle(app, FixedPostUpdate, seed);
    shuffle(app, FixedLast, seed);
}

/// The seeds a randomized test runs under: 4 in the fast profile, 32 in full.
pub fn seeds() -> Vec<u64> {
    let count = match Profile::from_env() {
        Profile::Fast => 4,
        Profile::Full => 32,
    };
    (0..count).map(|i| 0x5EED_0000 + i).collect()
}

/// Runs `test` once per seed. If one fails, the seed is printed so the failure
/// can be reproduced by calling `randomize_schedules` with it.
pub fn for_each_seed(mut test: impl FnMut(u64)) {
    for seed in seeds() {
        if let Err(panic) = catch_unwind(AssertUnwindSafe(|| test(seed))) {
            eprintln!("FAILED under schedule seed {seed:#x}");
            resume_unwind(panic);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn seeds_are_distinct_and_stable() {
        let s = seeds();
        assert!(s.len() >= 4);
        assert_eq!(s, seeds());
        let mut sorted = s.clone();
        sorted.dedup();
        assert_eq!(sorted.len(), s.len());
    }

    #[test]
    fn a_failing_seed_is_reported_by_propagating_the_panic() {
        let result = catch_unwind(|| for_each_seed(|_| panic!("boom")));
        assert!(result.is_err());
    }
}
