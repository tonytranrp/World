//! GPU time per pass, read back without stalling the frame.
//!
//! **Timestamps.** Where the device has `TIMESTAMP_QUERY`, each timed pass
//! writes a timestamp where it begins and where it ends (pass-boundary
//! writes). A frame's timestamps go to one slot of a small ring: after every
//! camera has drawn, the slot's queries are resolved and copied to its
//! read-back buffer, the buffer is mapped once the frame is submitted, and a
//! later frame collects whatever finished without waiting. With every slot
//! in flight a frame is simply not timed. This is always on.
//!
//! **Sampling.** wgpu exposes timestamps on Metal only where the GPU can
//! sample counters at stage boundaries, which Intel and AMD Macs (the floor
//! machine's Radeon Pro 5300M among them) cannot. There, when
//! `sample_every_frames` in `config/gpu_timing.ron` is not 0, one frame in
//! that many submits each timed pass on its own and waits for it: the wall
//! time of the pass minus that of an empty submission. That frame stalls,
//! so sampling is for measurement runs and off by default.
//!
//! Sums accumulate until the periodic log line takes them.

use std::fmt;
use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};
use std::time::Instant;

use bevy::prelude::*;
use bevy::render::MainWorld;
use bevy::render::renderer::{RenderContext, RenderDevice, RenderQueue};
use serde::{Deserialize, Serialize};
use wgpu::{
    Buffer, BufferDescriptor, BufferUsages, CommandEncoder, ComputePassTimestampWrites, Device,
    MapMode, QuerySet, QuerySetDescriptor, QueryType, Queue, RenderPassTimestampWrites,
};
use world_core::ConfigAsset;
use world_reload::Config;

/// The passes that are timed, in the order the log line lists them.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TimedPass {
    /// Primary rays (once per frame: the first 3D view that traces).
    Trace,
    /// Hits to the lit target, at the trace's size.
    Shade,
    /// The lit target stretched to the view's colour and depth.
    Composite,
    /// The brick cache's apply pass (frames with an upload).
    Apply,
    /// The brick cache's eviction pass (frames that evict).
    Evict,
}

/// Number of [`TimedPass`] variants.
pub const TIMED_PASSES: usize = 5;

impl TimedPass {
    pub const ALL: [Self; TIMED_PASSES] = [
        Self::Trace,
        Self::Shade,
        Self::Composite,
        Self::Apply,
        Self::Evict,
    ];

    pub fn label(self) -> &'static str {
        match self {
            Self::Trace => "trace",
            Self::Shade => "shade",
            Self::Composite => "composite",
            Self::Apply => "apply",
            Self::Evict => "evict",
        }
    }

    fn index(self) -> usize {
        self as usize
    }
}

/// Frames whose timestamps may be in flight at once. Results arrive two or
/// three frames late; a fourth slot keeps a slow frame from skipping one.
pub const TIMING_SLOTS: usize = 4;

/// Resolve destinations must be 256-byte aligned, so each pass's pair gets
/// its own 256 bytes of the resolve buffer.
const RESOLVE_STRIDE: u64 = wgpu::QUERY_RESOLVE_BUFFER_ALIGNMENT;
/// Two `u64` timestamps.
const PAIR_BYTES: u64 = 16;
/// A pass longer than this is a bogus read (a counter reset), not a time.
const MAX_PLAUSIBLE_MS: f64 = 10_000.0;

const MAP_PENDING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;

enum SlotState {
    Free,
    /// This frame's passes write here; bit `i` is set once pass `i` did.
    Recording {
        written: u8,
    },
    /// Resolved and copied, not yet submitted.
    Resolved {
        written: u8,
    },
    Mapping {
        written: u8,
        flag: Arc<AtomicU8>,
    },
}

struct Slot {
    readback: Buffer,
    state: SlotState,
}

/// One pass's GPU time over the frames it was timed in.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct PassTime {
    pub frames: u32,
    pub total_ms: f64,
    /// The least time: sampled timing adds the render thread's wake-up
    /// latency to some frames, so on a loaded machine this is the steadier
    /// figure.
    pub min_ms: f64,
    pub max_ms: f64,
}

impl PassTime {
    pub fn add(&mut self, ms: f64) {
        self.min_ms = if self.frames == 0 {
            ms
        } else {
            self.min_ms.min(ms)
        };
        self.frames += 1;
        self.total_ms += ms;
        self.max_ms = self.max_ms.max(ms);
    }

    pub fn mean_ms(&self) -> f64 {
        if self.frames == 0 {
            0.0
        } else {
            self.total_ms / f64::from(self.frames)
        }
    }
}

/// Pass times since the last report, and the per-frame sum of every timed
/// pass (over frames where at least one was read back).
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct GpuTimes {
    pub passes: [PassTime; TIMED_PASSES],
    pub sum: PassTime,
}

impl GpuTimes {
    /// Adds one frame's read-back: milliseconds per pass, `None` where the
    /// pass did not run.
    pub fn add_frame(&mut self, frame: [Option<f64>; TIMED_PASSES]) {
        let mut sum = None;
        for (time, ms) in self.passes.iter_mut().zip(frame) {
            if let Some(ms) = ms {
                time.add(ms);
                sum = Some(sum.unwrap_or(0.0) + ms);
            }
        }
        if let Some(sum) = sum {
            self.sum.add(sum);
        }
    }

    pub fn is_empty(&self) -> bool {
        self.sum.frames == 0
    }

    /// The times of one pass.
    pub fn pass(&self, pass: TimedPass) -> PassTime {
        self.passes[pass.index()]
    }
}

impl fmt::Display for GpuTimes {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{} frames timed, ms mean / min / max (frames): ",
            self.sum.frames
        )?;
        for pass in TimedPass::ALL {
            let t = self.passes[pass.index()];
            write!(
                f,
                "{} {:.2} / {:.2} / {:.2} ({}), ",
                pass.label(),
                t.mean_ms(),
                t.min_ms,
                t.max_ms,
                t.frames
            )?;
        }
        write!(
            f,
            "sum {:.2} / {:.2} / {:.2}",
            self.sum.mean_ms(),
            self.sum.min_ms,
            self.sum.max_ms
        )
    }
}

/// The query ring and the sums since the last report.
#[derive(Resource)]
pub struct GpuPassTimer {
    queries: QuerySet,
    resolve: Buffer,
    slots: Vec<Slot>,
    /// The slot this frame's passes write to, if one was free.
    current: Option<usize>,
    period_ns: f64,
    times: GpuTimes,
    /// Frames not timed because every slot was busy.
    pub skipped: u64,
    /// Mappings that failed and implausible reads.
    pub failed: u64,
}

impl GpuPassTimer {
    /// The timer, or `None` when the device lacks `TIMESTAMP_QUERY`.
    pub fn new(device: &Device, queue: &Queue) -> Option<Self> {
        if !device.features().contains(wgpu::Features::TIMESTAMP_QUERY) {
            return None;
        }
        let queries = device.create_query_set(&QuerySetDescriptor {
            label: Some("gpu_timing.queries"),
            ty: QueryType::Timestamp,
            count: (TIMING_SLOTS * TIMED_PASSES * 2) as u32,
        });
        let resolve = device.create_buffer(&BufferDescriptor {
            label: Some("gpu_timing.resolve"),
            size: (TIMING_SLOTS * TIMED_PASSES) as u64 * RESOLVE_STRIDE,
            usage: BufferUsages::QUERY_RESOLVE | BufferUsages::COPY_SRC,
            mapped_at_creation: false,
        });
        let slots = (0..TIMING_SLOTS)
            .map(|i| Slot {
                readback: device.create_buffer(&BufferDescriptor {
                    label: Some(&format!("gpu_timing.readback{i}")),
                    size: TIMED_PASSES as u64 * PAIR_BYTES,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: SlotState::Free,
            })
            .collect();
        Some(Self {
            queries,
            resolve,
            slots,
            current: None,
            period_ns: f64::from(queue.get_timestamp_period()),
            times: GpuTimes::default(),
            skipped: 0,
            failed: 0,
        })
    }

    /// Collects finished read-backs (without waiting) and starts a frame in
    /// a free slot. A frame left open (its passes never resolved) is reused.
    pub fn begin_frame(&mut self, device: &Device) {
        self.collect(device);
        if let Some(i) = self.current
            && matches!(self.slots[i].state, SlotState::Recording { .. })
        {
            self.slots[i].state = SlotState::Recording { written: 0 };
            return;
        }
        self.current = self
            .slots
            .iter()
            .position(|s| matches!(s.state, SlotState::Free));
        match self.current {
            Some(i) => self.slots[i].state = SlotState::Recording { written: 0 },
            None => self.skipped += 1,
        }
    }

    /// The query indices of `pass` this frame, claiming them; `None` when
    /// the frame is not timed or the pass already wrote this frame.
    fn claim(&mut self, pass: TimedPass) -> Option<u32> {
        let i = self.current?;
        let SlotState::Recording { written } = &mut self.slots[i].state else {
            return None;
        };
        let bit = 1u8 << pass.index();
        if *written & bit != 0 {
            return None;
        }
        *written |= bit;
        Some(((i * TIMED_PASSES + pass.index()) * 2) as u32)
    }

    /// Timestamp writes for a compute pass about to be recorded.
    pub fn compute_writes(&mut self, pass: TimedPass) -> Option<ComputePassTimestampWrites<'_>> {
        let first = self.claim(pass)?;
        Some(ComputePassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        })
    }

    /// Timestamp writes for a render pass about to be recorded.
    pub fn render_writes(&mut self, pass: TimedPass) -> Option<RenderPassTimestampWrites<'_>> {
        let first = self.claim(pass)?;
        Some(RenderPassTimestampWrites {
            query_set: &self.queries,
            beginning_of_pass_write_index: Some(first),
            end_of_pass_write_index: Some(first + 1),
        })
    }

    /// Resolves the passes this frame wrote and copies them to the slot's
    /// read-back buffer. Record after every timed pass.
    pub fn resolve(&mut self, encoder: &mut CommandEncoder) {
        let Some(i) = self.current.take() else {
            return;
        };
        let slot = &mut self.slots[i];
        let SlotState::Recording { written } = slot.state else {
            return;
        };
        if written == 0 {
            slot.state = SlotState::Free;
            return;
        }
        for pass in TimedPass::ALL {
            if written & (1 << pass.index()) == 0 {
                continue;
            }
            let at = (i * TIMED_PASSES + pass.index()) as u32;
            let offset = u64::from(at) * RESOLVE_STRIDE;
            encoder.resolve_query_set(&self.queries, at * 2..at * 2 + 2, &self.resolve, offset);
            encoder.copy_buffer_to_buffer(
                &self.resolve,
                offset,
                &slot.readback,
                pass.index() as u64 * PAIR_BYTES,
                PAIR_BYTES,
            );
        }
        slot.state = SlotState::Resolved { written };
    }

    /// Maps the slots resolved this frame. Call after submission.
    pub fn after_submit(&mut self) {
        for slot in &mut self.slots {
            if let SlotState::Resolved { written } = slot.state {
                let flag = Arc::new(AtomicU8::new(MAP_PENDING));
                let done = flag.clone();
                slot.readback
                    .slice(..)
                    .map_async(MapMode::Read, move |result| {
                        let v = if result.is_ok() { MAP_DONE } else { MAP_FAILED };
                        done.store(v, Ordering::Release);
                    });
                slot.state = SlotState::Mapping { written, flag };
            }
        }
    }

    /// Polls without waiting and adds every finished frame to the sums.
    pub fn collect(&mut self, device: &Device) {
        let _ = device.poll(wgpu::PollType::Poll);
        for slot in &mut self.slots {
            let SlotState::Mapping { written, ref flag } = slot.state else {
                continue;
            };
            match flag.load(Ordering::Acquire) {
                MAP_DONE => {
                    let ticks: Option<Vec<u64>> = slot
                        .readback
                        .slice(..)
                        .get_mapped_range()
                        .ok()
                        .map(|view| bytemuck::pod_collect_to_vec::<u8, u64>(&view));
                    slot.readback.unmap();
                    slot.state = SlotState::Free;
                    let Some(ticks) = ticks else {
                        self.failed += 1;
                        continue;
                    };
                    let mut frame = [None; TIMED_PASSES];
                    for pass in TimedPass::ALL {
                        let p = pass.index();
                        if written & (1 << p) == 0 {
                            continue;
                        }
                        let (start, end) = (ticks[2 * p], ticks[2 * p + 1]);
                        let ms = end.wrapping_sub(start) as f64 * self.period_ns / 1e6;
                        if end < start || ms > MAX_PLAUSIBLE_MS {
                            self.failed += 1;
                        } else {
                            frame[p] = Some(ms);
                        }
                    }
                    self.times.add_frame(frame);
                }
                MAP_FAILED => {
                    self.failed += 1;
                    slot.state = SlotState::Free;
                }
                _ => {}
            }
        }
    }

    /// The sums since the last call, or `None` if nothing was read back.
    pub fn take(&mut self) -> Option<GpuTimes> {
        if self.times.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.times))
    }
}

/// GPU timing settings, loaded from `assets/config/gpu_timing.ron`. The
/// default (0) is off.
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GpuTimingConfig {
    /// On devices without timestamp queries, time the passes of one frame in
    /// this many by submitting each alone and waiting for it (that frame
    /// stalls). `0` is off. Ignored where timestamps exist. `0..=1000000`.
    pub sample_every_frames: u32,
}

impl ConfigAsset for GpuTimingConfig {
    const NAME: &'static str = "gpu_timing";
    const PATH: &'static str = "config/gpu_timing.ron";

    fn validate(&self) -> Result<(), Vec<String>> {
        if self.sample_every_frames > 1_000_000 {
            return Err(vec![format!(
                "sample_every_frames = {} is outside 0..=1000000",
                self.sample_every_frames
            )]);
        }
        Ok(())
    }
}

/// What the rays of sampled frames did, from their hit records: how many
/// traversal steps they took and how they ended.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct RayStats {
    pub rays: u64,
    pub steps: u64,
    pub max_steps: u32,
    /// Rays that took more than 64 and more than 256 steps.
    pub over_64: u64,
    pub over_256: u64,
    /// Rays that hit nothing (sky), and their steps.
    pub misses: u64,
    pub miss_steps: u64,
    /// Hits decided at a coarse cell by the pixel footprint, or at a node
    /// whose children are not resident.
    pub footprint: u64,
    pub residency: u64,
    pub exhausted: u64,
}

impl RayStats {
    /// Adds the hit records of one traced view (`HIT_WORDS` words a pixel).
    pub fn add_records(&mut self, words: &[u32]) {
        use crate::gbuffer::{
            HIT_FLAG_EXHAUSTED, HIT_FLAG_FOOTPRINT, HIT_FLAG_HIT, HIT_FLAG_RESIDENCY, HIT_WORDS,
        };
        for record in words.chunks_exact(HIT_WORDS as usize) {
            let flags = record[3] >> 24;
            let steps = record[6];
            self.rays += 1;
            self.steps += u64::from(steps);
            self.max_steps = self.max_steps.max(steps);
            self.over_64 += u64::from(steps > 64);
            self.over_256 += u64::from(steps > 256);
            if flags & HIT_FLAG_HIT == 0 {
                self.misses += 1;
                self.miss_steps += u64::from(steps);
            }
            self.footprint += u64::from(flags & HIT_FLAG_FOOTPRINT != 0);
            self.residency += u64::from(flags & HIT_FLAG_RESIDENCY != 0);
            self.exhausted += u64::from(flags & HIT_FLAG_EXHAUSTED != 0);
        }
    }
}

impl fmt::Display for RayStats {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let rays = self.rays.max(1) as f64;
        let pct = |n: u64| 100.0 * n as f64 / rays;
        write!(
            f,
            "{} rays, {:.1} steps a ray (max {}), {:.1}% over 64, {:.2}% over 256; \
             {:.1}% sky ({:.1} steps each), {:.1}% footprint hits, {:.2}% residency hits, \
             {:.3}% exhausted",
            self.rays,
            self.steps as f64 / rays,
            self.max_steps,
            pct(self.over_64),
            pct(self.over_256),
            pct(self.misses),
            self.miss_steps as f64 / self.misses.max(1) as f64,
            pct(self.footprint),
            pct(self.residency),
            pct(self.exhausted),
        )
    }
}

/// Wall-clock timing of passes submitted alone, for devices without
/// timestamp queries. Only frames chosen by [`GpuTimingConfig`] are timed.
#[derive(Resource, Default)]
pub struct SampledPassTimer {
    /// `sample_every_frames`, copied from the main world each frame.
    pub every: u32,
    frames: u64,
    sampling: bool,
    frame: [Option<f64>; TIMED_PASSES],
    times: GpuTimes,
    rays: RayStats,
}

impl SampledPassTimer {
    /// Starts a frame: decides whether its passes are timed.
    pub fn begin_frame(&mut self) {
        self.finish_frame();
        self.frames += 1;
        self.sampling = self.every > 0 && self.frames.is_multiple_of(u64::from(self.every));
    }

    /// Adds the last sampled frame's passes to the sums.
    fn finish_frame(&mut self) {
        if self.sampling && self.frame.iter().any(Option::is_some) {
            self.times.add_frame(self.frame);
        }
        self.frame = [None; TIMED_PASSES];
    }

    /// Whether this frame's pass `pass` should be recorded alone and timed
    /// with [`Self::time_alone`].
    pub fn wants(&self, pass: TimedPass) -> bool {
        self.sampling && self.frame[pass.index()].is_none()
    }

    /// Submits everything recorded so far (`flush`), waits for it, then
    /// submits `encoder` alone and waits again: the pass's wall time minus
    /// that of an empty submission is its GPU time. Stalls the render thread.
    pub fn time_alone(
        &mut self,
        pass: TimedPass,
        device: &Device,
        queue: &Queue,
        flush: impl FnOnce(),
        encoder: CommandEncoder,
    ) {
        let wait = || {
            let _ = device.poll(wgpu::PollType::wait_indefinitely());
        };
        flush();
        wait();
        let empty = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu_timing.empty"),
        });
        let start = Instant::now();
        queue.submit([empty.finish()]);
        wait();
        let overhead = start.elapsed().as_secs_f64() * 1e3;
        let start = Instant::now();
        queue.submit([encoder.finish()]);
        wait();
        let ms = start.elapsed().as_secs_f64() * 1e3;
        self.frame[pass.index()] = Some((ms - overhead).max(0.0));
    }

    /// Copies a traced view's hit records back and adds them to the ray
    /// statistics, waiting for the copy (sampled frames only).
    pub fn read_rays(&mut self, device: &Device, queue: &Queue, hits: &Buffer) {
        let staging = device.create_buffer(&BufferDescriptor {
            label: Some("gpu_timing.hits_readback"),
            size: hits.size(),
            usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let mut encoder = device.create_command_encoder(&wgpu::CommandEncoderDescriptor {
            label: Some("gpu_timing.hits_copy"),
        });
        encoder.copy_buffer_to_buffer(hits, 0, &staging, 0, hits.size());
        queue.submit([encoder.finish()]);
        let slice = staging.slice(..);
        slice.map_async(MapMode::Read, |_| {});
        let _ = device.poll(wgpu::PollType::wait_indefinitely());
        if let Ok(view) = slice.get_mapped_range() {
            self.rays
                .add_records(&bytemuck::pod_collect_to_vec::<u8, u32>(&view));
        }
        staging.unmap();
    }

    /// The sums since the last call, or `None` if nothing was sampled.
    pub fn take(&mut self) -> Option<GpuTimes> {
        if self.times.is_empty() {
            return None;
        }
        Some(std::mem::take(&mut self.times))
    }

    /// The ray statistics since the last call, or `None` if none were read.
    pub fn take_rays(&mut self) -> Option<RayStats> {
        if self.rays.rays == 0 {
            return None;
        }
        Some(std::mem::take(&mut self.rays))
    }
}

/// Creates the timestamp timer when the device supports timestamp queries,
/// and the sampling timer otherwise.
pub fn init_gpu_timer(mut commands: Commands, device: Res<RenderDevice>, queue: Res<RenderQueue>) {
    match GpuPassTimer::new(device.wgpu_device(), &queue) {
        Some(timer) => commands.insert_resource(timer),
        None => {
            debug!("GPU pass timestamps off: the device has no TIMESTAMP_QUERY");
            commands.insert_resource(SampledPassTimer::default());
        }
    }
}

/// Copies the sampling interval into the render world.
pub fn extract_gpu_timing(main_world: Res<MainWorld>, sampled: Option<ResMut<SampledPassTimer>>) {
    if let (Some(mut sampled), Some(config)) = (
        sampled,
        main_world.get_resource::<Config<GpuTimingConfig>>(),
    ) && sampled.every != config.sample_every_frames
    {
        sampled.every = config.sample_every_frames;
    }
}

/// Starts the frame's timing (`Render`, before any pass is recorded).
pub fn begin_gpu_timing(
    timer: Option<ResMut<GpuPassTimer>>,
    sampled: Option<ResMut<SampledPassTimer>>,
    device: Res<RenderDevice>,
) {
    if let Some(mut timer) = timer {
        timer.begin_frame(device.wgpu_device());
    }
    if let Some(mut sampled) = sampled {
        sampled.begin_frame();
    }
}

/// Resolves the frame's timestamps (`RenderGraph`, after every camera).
pub fn resolve_gpu_timing(timer: Option<ResMut<GpuPassTimer>>, mut ctx: RenderContext) {
    if let Some(mut timer) = timer {
        timer.resolve(ctx.command_encoder());
    }
}

/// Maps the slot resolved this frame (`RenderGraph`, after submission).
pub fn map_gpu_timing(timer: Option<ResMut<GpuPassTimer>>) {
    if let Some(mut timer) = timer {
        timer.after_submit();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_default_is_valid_and_off() {
        let c = GpuTimingConfig::default();
        assert_eq!(c.validate(), Ok(()));
        assert_eq!(c.sample_every_frames, 0);
        let bad = GpuTimingConfig {
            sample_every_frames: 1_000_001,
        };
        assert_eq!(bad.validate().unwrap_err().len(), 1);
    }

    #[test]
    fn sampling_picks_one_frame_in_every_and_each_pass_once() {
        let mut s = SampledPassTimer::default();
        s.begin_frame();
        assert!(!s.wants(TimedPass::Trace), "0 is off");
        s.every = 3;
        let picked: Vec<bool> = (0..6)
            .map(|_| {
                s.begin_frame();
                s.wants(TimedPass::Trace)
            })
            .collect();
        // Frames 2 to 7: frames 3 and 6 are sampled.
        assert_eq!(picked, [false, true, false, false, true, false]);
        s.every = 1;
        s.begin_frame();
        s.frame[TimedPass::Trace.index()] = Some(2.0);
        assert!(!s.wants(TimedPass::Trace), "timed once per frame");
        assert!(s.wants(TimedPass::Composite));
        s.begin_frame();
        let times = s.take().unwrap();
        assert_eq!(times.passes[TimedPass::Trace.index()].frames, 1);
        assert_eq!(times.sum.mean_ms(), 2.0);
        assert_eq!(s.take(), None);
    }

    #[test]
    fn frames_add_up_per_pass_and_in_sum() {
        let mut t = GpuTimes::default();
        assert!(t.is_empty());
        t.add_frame([Some(4.0), None, Some(0.5), None, None]);
        t.add_frame([Some(6.0), None, Some(0.5), Some(1.0), None]);
        t.add_frame([None; TIMED_PASSES]);
        assert_eq!(t.sum.frames, 2);
        assert_eq!(t.sum.mean_ms(), 6.0);
        assert_eq!(t.sum.max_ms, 7.5);
        let trace = t.passes[TimedPass::Trace.index()];
        assert_eq!(
            (trace.frames, trace.mean_ms(), trace.min_ms, trace.max_ms),
            (2, 5.0, 4.0, 6.0)
        );
        assert_eq!(t.passes[TimedPass::Apply.index()].frames, 1);
        assert_eq!(t.passes[TimedPass::Evict.index()].mean_ms(), 0.0);
        assert_eq!(
            t.to_string(),
            "2 frames timed, ms mean / min / max (frames): trace 5.00 / 4.00 / 6.00 (2), \
             shade 0.00 / 0.00 / 0.00 (0), composite 0.50 / 0.50 / 0.50 (2), apply 1.00 / 1.00 / 1.00 (1), \
             evict 0.00 / 0.00 / 0.00 (0), sum 6.00 / 4.50 / 7.50"
        );
    }
}
