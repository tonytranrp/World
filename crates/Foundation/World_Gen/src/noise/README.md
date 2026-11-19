# Noise and Hashing

## Purpose

This module is the foundation of determinism for the whole generator. It supplies the hashing, random streams and noise functions every other stage builds on, written so that results are identical on every platform, compiler setting and CPU. Platform math libraries and ambiguous floating-point behavior are the main threat to a save moving between Windows and macOS, and this module is where that threat is contained.

## Scope

- Integer hash functions that map seed, coordinate and a domain tag to well-mixed bits, and small random streams built on them.
- Gradient and value noise in two and three dimensions, using integer lattice hashing.
- Fractal Brownian motion, ridged and billow variants, and domain warping composed from the basic noise.
- Worley or cellular noise for features such as cell-based placement.
- Fixed-point or carefully ordered float arithmetic, and explicit implementations of any needed functions such as smoothing curves and trigonometric approximations, instead of calling platform libraries.
- Sampling utilities for noise over regions, designed to be vectorizable without changing results.

## Out of scope

- Terrain shaping, biome logic and any use of noise for a specific purpose: continental, regional, props and surface.
- GPU noise. A GPU implementation may exist elsewhere for visual-only work and must never feed authoritative results.
- General math or linear algebra libraries.

## Relationships

A leaf module with no dependency on other World_Gen modules and only the coordinate types of World_Core. Every other stage depends on it.

## Rules

- Authoritative paths use integer math, or basic IEEE-754 operations (add, subtract, multiply, divide, square root) in a fixed order.
- No platform math library calls for sine, cosine, exponent, logarithm or power in authoritative paths, and no fused multiply-add unless it is performed explicitly everywhere.
- No fast-math options, no dependence on SIMD width or thread count.
- Domain separation: different uses of the same seed and coordinate get different hash domains so streams never correlate.
- Each primitive has a golden test of exact output values.
