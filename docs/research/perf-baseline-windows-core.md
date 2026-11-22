# Voxel core performance baseline (Windows)

Baseline of World_Core's CPU memory and read/edit/build times before the
`perf/core` work, measured on `perf/integration` at f65d385 (decisions up to
0026). Later decisions (0027 on) compare against these figures.

Machine: Ryzen 9 5980HX (8 cores, 16 threads, 16 MiB L3), 15 GB RAM,
Windows 11, release (bench) profile. Two other cargo builds shared the
machine during every run, so wall-time figures move by 10 to 50% between
runs of unchanged code (Criterion's "change" lines against an earlier run
showed -55% to +24% on untouched benches); compare runs made back to back.
The gungraun instruction-count bench needs Linux and Valgrind and was not
run.

## Bytes of real chunks

`cargo test --release -p world_gen --test chunk_bytes -- --ignored --nocapture`
(new; the same 107 full-detail chunks as decisions 0021 and 0025, and the
coarse ring of decision 0020 at cell level 2, footprints 64 to 384 m, which
now counts 3,604 chunks, not 0020's 2,022: the ring takes every chunk the
surface crosses and one below).

| | Full detail, 107 chunks | Coarse ring, 3,604 chunks |
|---|---|---|
| Live bytes | 154.6 MiB (median 1,464 KiB a chunk; p10 639, p90 2,333) | 155.8 MiB (median 36 KiB; p90 101) |
| Upper nodes, child words | 46,842 nodes, 2,286,113 words (9.4 MiB) | 801,258 nodes, 37,640,005 words (156 MiB: all of it) |
| Bricks, long headers | 698,941, 513,557 | none |
| Default full leaves (no bytes) | 21,785,042 | |
| Default partial leaves (8 B) | 5,219,942 (39.8 MiB) | |
| Other full leaves (4 B) | 2,011,900 (7.7 MiB) | |
| Single-material records (12 B) | 894,341 (10.2 MiB) | |
| Palette records (12 B) | 2,834,915 (32.4 MiB) | |
| Palette block words, all 1-bit | 8,504,745 (32.4 MiB) | |

`cargo test --release -p world_render --test terrain_bytes -- --ignored --nocapture`
gives the same CPU figures and the GPU block bytes (145.5 MiB after 0025);
it was not rerun here because it builds World_Render in release.

## `cargo bench -p world_core --bench memory`

The bench counted every leaf as a 16-byte node although leaves are records
inside their brick's words, so its "live" figure exceeded the bytes held.
Fixed: live bytes are `branches * 16 + words * 4`; it now also prints
`memory_breakdown()` and a terrain built bottom-up (the builder's layout).

| Scene (bench terrain 512 x 512, 309,695 surface voxels) | Held | Live | Per surface voxel (held, live) | Free share of words |
|---|---|---|---|---|
| Built by box fills | 1.62 MiB | 1.19 MiB | 5.50, 4.02 B | 24.1% |
| Built bottom-up | 1.22 MiB | 1.19 MiB | 4.14, 4.02 B | 0.3% |
| After 1,000,000 churn edits | 2.86 MiB | 1.34 MiB | 9.68, 4.55 B | 52.6% |
| Dense 128^3, per voxel | 2.31 MiB | 2.02 MiB | 1.16, 1.01 B | 11.2% |

The old "7.5 bytes per surface voxel" (decision 0006) predates 0021 and 0025;
the bench terrain is now 4.0 live bytes per surface voxel, real terrain about
the same order (it has caves, overhangs and three cover materials).

## `cargo bench -p world_core --bench workloads`

| Bench | Time |
|---|---|
| overwrite_dense/set_10k | 15.3 ms |
| carve_solid/sphere_r16_set_air | 6.57 ms |
| carve_solid/tunnel_32_boxes_fill_air | 281 µs |
| coherent_reads/terrain_line_512 | 20.3 µs (46.0 in a first run) |
| coherent_reads/terrain_surface_brick_16 | 228 µs (293) |
| coherent_reads/dense_brick_16 | 261 µs (208) |
| coherent_reads/bottom_up_line_512 (new) | 24.0 µs |
| coherent_reads/bottom_up_surface_brick_16 (new) | 225 µs |
| coherent_reads/terrain_scattered_10k (new) | 998 µs |
| coherent_reads/bottom_up_scattered_10k (new) | 1,018 µs |
| apply_1k/clustered_16 | 71.7 µs |
| apply_1k/scattered_128 | 524 µs |
| snapshot_then_edit/no_snapshot_1_edit | 1.35 µs |
| snapshot_then_edit/snapshot_1_edit | 3.74 µs |
| snapshot_then_edit/snapshot_1k_scattered_edits | 965 µs |
| serialize/encode_terrain_512, decode | 21.2 ms, 23.7 ms |
| serialize/encode_dense_128, decode | 28.8 ms, 24.0 ms |
| chunks/take_and_insert_dense_chunk | 30.6 µs |
| chunks/compact_fragmented_dense_chunk | 30.4 ms |
| chunks/compress_Lz4, decompress | 4.54 ms, 1.17 ms |
| chunks/compress_Zstd, decompress | 100 ms, 10.1 ms |

## `cargo bench -p world_gen --bench terrain -- terrain_tree`

| Bench | Time |
|---|---|
| chunk_columns_and_build | 55.9 ms |
| chunk_build_only | 32.8 ms |
| caves_surface_columns_grid_and_build | 72.1 ms |
| caves_underground_columns_grid_and_build | 74.2 ms |
| air_columns_and_build | 19.1 ms |
| caves_underground_build_only | 59.4 ms |

## `cargo bench -p world_gen --bench chunk_reads` (new)

25 generated chunks (the surface chunks and one below of a 3 x 3 footprint
at chunk 516..518, 508..510), 35.7 MiB held, built with `build_region` as
the workers build them.

| Bench | Time |
|---|---|
| get_100k_near_surface (`VoxelTree::get`, within 16 voxels of the surface) | 26.8 ms |
| cursor_100k_near_surface (the same through one `TreeCursor`) | 28.4 ms |
| cursor_rays_256 (rays from 64 voxels up, one voxel a step, to the first hit) | 358 µs |
| walk_every_brick (every leaf of every brick through `NodeView`, as the GPU encoder reads them) | 293 ms |
| crater_r12_and_restore (`apply`, every voxel of a radius-12 sphere, about 7,200, to air and back) | 900 µs |
| scattered_1k_and_restore (`apply`, 1,000 voxels to stone and back) | 2.95 ms |
