# Feedback

## Purpose

Implements the channel by which rays tell the CPU what they could not find. When traversal reaches a node or brick that is not resident in the pool, the shader appends a request to a feedback buffer. This folder owns that buffer, its readback, and its decoding into a deduplicated request list.

## Scope

- The GPU feedback buffer: layout of a request (tree node identity, requested mip level, a priority hint such as pixel count or distance), the atomic counter, and overflow behavior.
- Clearing and double-buffering so the readback of frame N does not stall frame N+1.
- Asynchronous readback to the CPU and decoding into request records.
- Deduplication, merging and ranking of requests, since many pixels ask for the same brick. Output is an ordered list for `stream`.
- Residency "touch" information, so bricks that are being hit are marked used for LRU purposes.

## Out of scope

- Fulfilling requests, uploading data and the MB budget: `stream`.
- Choosing victims: `evict`, which consumes the touch information produced here.
- Emitting requests: the traversal WESL code under `assets/shaders/traversal`; this folder defines the contract that code follows.
- Pool storage and slots: `pool`.

## Relationships

Depends on `pool` for node identities and on World_Core's identifiers for tree nodes. Feeds `stream` and `evict`. Sits between the GPU traversal passes and the CPU side of `brick_cache`, scheduled by `graph`.

## Rules

- The buffer is bounded; overflow drops low-priority requests and never blocks or errors. Missing data resolves itself over later frames.
- Readback is latency tolerant: results are one or more frames old and consumers must assume the camera moved.
- Request encoding is generated from World_Core constants so shader and Rust agree.
- Decoding and deduplication are deterministic for a given buffer content, so they can be tested without a GPU.
- Writes from shaders use atomics only; no ordering between rays is assumed.
