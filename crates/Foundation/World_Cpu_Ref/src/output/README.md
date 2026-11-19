# Output

## Purpose

This module holds the results of reference rendering and the tools for looking at and comparing them. It defines the reference G-buffer and everything needed to turn it into a PNG for a human or into a verdict for a test.

## Scope

- The reference G-buffer: per-pixel depth, normal, material id and motion vector, using the same encodings and conventions the GPU primary pass writes.
- A shaded color image built from the G-buffer for human inspection and golden images.
- PNG encoding of images and of debug visualizations such as normals, depth ramps and material colors.
- Comparison utilities: exact per-pixel equality of hits and material ids, depth comparison with a stated tolerance, color comparison with a stated tolerance, and a difference image that highlights mismatches.
- Summary reports for tests: count of mismatched pixels, first mismatch location and a stable hash of a buffer.
- Loading previously saved reference buffers for comparison against a GPU capture.

## Out of scope

- Casting rays: trace.
- Producing rays or the camera: camera.
- Windowing, display and live preview.
- Golden images of full Bevy frames and the tolerance policy for them: World_Render.
- General-purpose image processing beyond what comparison needs.

## Relationships

It consumes results from the trace module and depends on the camera for depth conventions. The tests folder and World_Render's tests use its comparison API. Tools that dump images call its PNG writing.

## Rules

- Buffers are plain arrays with explicit width, height and element encodings; there is no hidden padding or GPU-specific layout.
- Hit data is compared exactly. A tolerance is only allowed on derived values, and every tolerance is a named constant with a reason.
- Written images are deterministic byte for byte.
- Comparison results are informative: a failure must say where and by how much, not just that it failed.
- Output never modifies the input buffers.
