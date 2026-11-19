# World_Render Benches

## Purpose

Criterion benchmarks for the CPU-side hot paths of the renderer. They exist so that regressions in per-frame work are caught by CI before they show up as dropped frames, especially on the Mac floor where the CPU budget is tight.

## Scope

- Feedback processing: deduplicating and sorting missing-brick requests read back from the GPU, at realistic request counts per frame.
- Streaming and packing: serializing World_Core bricks and nodes into pool upload layouts, and the cost of staying under the MB-per-frame upload budget.
- LRU eviction and pool allocation bookkeeping under churn, such as a camera moving through terrain.
- Dynamic-object preparation: building oriented-box instance data and the small BVH over dynamic-object boxes used by reflections.
- Mip selection and tree-to-GPU mirroring of edits, since destruction edits constantly.

## Out of scope

- GPU timings. Those come from timestamp queries and tracing in the debug module, because Criterion cannot time a GPU frame faithfully.
- Tree edit and CPU traversal benchmarks: World_Core and World_Cpu_Ref.
- Correctness checks of any kind: `tests`.

## Relationships

Depends on world_render internals exposed for benching, World_Core for realistic input data, and Criterion. Nothing depends on it. CI compares results against thresholds and fails on regressions.

## Rules

- Benchmarks use deterministic generated input so runs are comparable.
- Each benchmark names the per-frame budget it protects, so a failure is interpretable.
- Measure single-threaded and at the capped worker count used by `--mac-floor`.
- Benchmarks must not require a window or GPU.
