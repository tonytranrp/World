//! A ring of read-back buffers that never blocks the render thread.
//!
//! Each frame the request block (and, after an eviction pass, the victim
//! block) is copied into a free slot. After the frame's submission the slot
//! is mapped asynchronously; a later frame polls the device without waiting
//! and collects whatever finished. With every slot in flight a frame's
//! feedback is simply skipped and counted: missing data resolves itself over
//! later frames.

use std::sync::Arc;
use std::sync::atomic::{AtomicU8, Ordering};

use wgpu::{Buffer, BufferDescriptor, BufferUsages, CommandEncoder, Device, MapMode};

/// Slots in the ring (decision 0007: three).
pub const RING_SLOTS: usize = 3;

const MAP_PENDING: u8 = 0;
const MAP_DONE: u8 = 1;
const MAP_FAILED: u8 = 2;

enum SlotState {
    Free,
    /// Copies recorded this frame, not yet submitted.
    Copied {
        frame: u32,
        victims: bool,
    },
    /// Mapping requested; the flag says when it finished.
    Mapping {
        frame: u32,
        victims: bool,
        flag: Arc<AtomicU8>,
    },
}

struct Slot {
    buffer: Buffer,
    state: SlotState,
}

/// One frame's feedback, copied out of a mapped slot.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Readback {
    pub frame: u32,
    pub requests: Vec<u32>,
    /// Empty when no eviction pass ran that frame.
    pub victims: Vec<u32>,
}

/// The ring and its counters.
pub struct ReadbackRing {
    slots: Vec<Slot>,
    request_words: u32,
    victim_words: u32,
    /// Frames whose feedback was skipped because every slot was busy.
    pub skipped: u64,
    /// Mappings that failed (device lost, for one).
    pub failed: u64,
}

impl ReadbackRing {
    pub fn new(device: &Device, request_words: u32, victim_words: u32) -> Self {
        let size = u64::from(request_words + victim_words) * 4;
        let slots = (0..RING_SLOTS)
            .map(|i| Slot {
                buffer: device.create_buffer(&BufferDescriptor {
                    label: Some(&format!("brick_cache.readback{i}")),
                    size,
                    usage: BufferUsages::MAP_READ | BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }),
                state: SlotState::Free,
            })
            .collect();
        Self {
            slots,
            request_words,
            victim_words,
            skipped: 0,
            failed: 0,
        }
    }

    /// Whether a victim block is on its way back.
    pub fn victims_in_flight(&self) -> bool {
        self.slots.iter().any(|s| {
            matches!(
                s.state,
                SlotState::Copied { victims: true, .. } | SlotState::Mapping { victims: true, .. }
            )
        })
    }

    /// Copies this frame's blocks into a free slot. Returns false (and counts
    /// a skip) when every slot is busy.
    pub fn record_copy(
        &mut self,
        encoder: &mut CommandEncoder,
        feedback: &Buffer,
        victims: Option<&Buffer>,
        frame: u32,
    ) -> bool {
        let Some(slot) = self
            .slots
            .iter_mut()
            .find(|s| matches!(s.state, SlotState::Free))
        else {
            self.skipped += 1;
            return false;
        };
        let req_bytes = u64::from(self.request_words) * 4;
        encoder.copy_buffer_to_buffer(feedback, 0, &slot.buffer, 0, req_bytes);
        if let Some(v) = victims {
            encoder.copy_buffer_to_buffer(
                v,
                0,
                &slot.buffer,
                req_bytes,
                u64::from(self.victim_words) * 4,
            );
        }
        slot.state = SlotState::Copied {
            frame,
            victims: victims.is_some(),
        };
        true
    }

    /// Requests mapping of the slots copied this frame. Call after the
    /// frame's command buffers were submitted.
    pub fn after_submit(&mut self) {
        for slot in &mut self.slots {
            if let SlotState::Copied { frame, victims } = slot.state {
                let flag = Arc::new(AtomicU8::new(MAP_PENDING));
                let done = flag.clone();
                slot.buffer
                    .slice(..)
                    .map_async(MapMode::Read, move |result| {
                        let v = if result.is_ok() { MAP_DONE } else { MAP_FAILED };
                        done.store(v, Ordering::Release);
                    });
                slot.state = SlotState::Mapping {
                    frame,
                    victims,
                    flag,
                };
            }
        }
    }

    /// Polls without waiting and returns finished read-backs, oldest first.
    pub fn collect(&mut self, device: &Device) -> Vec<Readback> {
        let _ = device.poll(wgpu::PollType::Poll);
        let mut out = Vec::new();
        for slot in &mut self.slots {
            let SlotState::Mapping {
                frame,
                victims,
                ref flag,
            } = slot.state
            else {
                continue;
            };
            match flag.load(Ordering::Acquire) {
                MAP_DONE => {
                    let words: Option<Vec<u32>> = slot
                        .buffer
                        .slice(..)
                        .get_mapped_range()
                        .ok()
                        .map(|view| bytemuck::pod_collect_to_vec::<u8, u32>(&view));
                    slot.buffer.unmap();
                    let Some(words) = words else {
                        self.failed += 1;
                        slot.state = SlotState::Free;
                        continue;
                    };
                    let split = (self.request_words as usize).min(words.len());
                    out.push(Readback {
                        frame,
                        requests: words[..split].to_vec(),
                        victims: if victims {
                            words[split..].to_vec()
                        } else {
                            Vec::new()
                        },
                    });
                    slot.state = SlotState::Free;
                }
                MAP_FAILED => {
                    self.failed += 1;
                    slot.state = SlotState::Free;
                }
                _ => {}
            }
        }
        out.sort_by_key(|r| r.frame);
        out
    }
}
