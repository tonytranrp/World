//! A headless Bevy `App` that tests step by hand.

use bevy::app::{App, Plugins};
use bevy::ecs::world::World;
use bevy::prelude::MinimalPlugins;

use crate::schedule::randomize_schedules;

/// The condition given to [`HeadlessApp::run_until`] never became true.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StepsExhausted {
    pub steps: usize,
}

/// An `App` with `MinimalPlugins` and no window, runner or GPU.
///
/// The app is finished lazily on the first step, so plugins and schedule
/// randomization can still be configured after `new`.
pub struct HeadlessApp {
    app: App,
    ready: bool,
}

impl Default for HeadlessApp {
    fn default() -> Self {
        Self::new()
    }
}

impl HeadlessApp {
    pub fn new() -> Self {
        let mut app = App::new();
        app.add_plugins(MinimalPlugins);
        Self { app, ready: false }
    }

    /// Shuffles system order with `seed`. Must be called before the first step.
    pub fn randomized(mut self, seed: u64) -> Self {
        assert!(!self.ready, "randomize schedules before the first step");
        randomize_schedules(&mut self.app, seed);
        self
    }

    pub fn add_plugins<M>(&mut self, plugins: impl Plugins<M>) -> &mut Self {
        assert!(!self.ready, "add plugins before the first step");
        self.app.add_plugins(plugins);
        self
    }

    /// The underlying app, for adding systems and resources.
    pub fn app_mut(&mut self) -> &mut App {
        &mut self.app
    }

    pub fn world(&self) -> &World {
        self.app.world()
    }

    pub fn world_mut(&mut self) -> &mut World {
        self.app.world_mut()
    }

    fn ensure_ready(&mut self) {
        if !self.ready {
            self.app.finish();
            self.app.cleanup();
            self.ready = true;
        }
    }

    /// Runs `count` frames.
    pub fn step(&mut self, count: usize) -> &mut Self {
        self.ensure_ready();
        for _ in 0..count {
            self.app.update();
        }
        self
    }

    /// Steps one frame at a time until `done` returns true. Returns the number
    /// of frames it took, or [`StepsExhausted`] after `max_steps`.
    pub fn run_until(
        &mut self,
        max_steps: usize,
        mut done: impl FnMut(&World) -> bool,
    ) -> Result<usize, StepsExhausted> {
        self.ensure_ready();
        for step in 0..max_steps {
            if done(self.app.world()) {
                return Ok(step);
            }
            self.app.update();
        }
        if done(self.app.world()) {
            Ok(max_steps)
        } else {
            Err(StepsExhausted { steps: max_steps })
        }
    }
}
