# PROTOTYPE — runtime feasibility

This branch contains throwaway code used to answer one question:

> Can a Rust implementation derived from featherbar render the approved two-line CPU/RAM indicator in one macOS status item, sample the approved metrics, and remain architecturally idle-friendly on Apple Silicon?

It is not production Statlet code and must not be merged into `main`.

## Run

```sh
cargo run --release
```

The status item shows global CPU on the first line and the approved RAM formula on the second. Samples are also printed to stderr so the observed state is explicit.

## Lineage

The event-loop and image-rendering approach is derived from featherbar commit `90ab504b025db15665ce5d97b8ae4d4cdeb47dc3`, licensed under Apache 2.0:

<https://github.com/nim444/featherbar/tree/90ab504b025db15665ce5d97b8ae4d4cdeb47dc3>

## Environment

- Apple M4, arm64
- macOS 26.5.2 (25F84)
- Rust 1.96.0
- upstream featherbar 0.2.1 at `90ab504b025db15665ce5d97b8ae4d4cdeb47dc3`

## Findings

**Verdict: feasible.**

- Both upstream featherbar and this prototype compile and run on the target Mac.
- The prototype renders `C` and `R` simultaneously on two fixed-width lines in one status item. Removing power and temperature makes it visibly narrower than upstream.
- CPU values matched upstream during the same samples and remained normalized to 0–100%.
- The approved RAM formula cannot reuse `sysinfo::System::used_memory()`. On macOS sysinfo includes speculative pages and omits inactive app pages; the prototype instead calculates apps + wired + compressed while subtracting purgeable and external cache.
- The custom RAM value differed from upstream by roughly 1–5 percentage points during observation, confirming that the semantic choice is user-visible.
- A short idle observation showed 0.0–0.2% CPU and a 15 MB physical footprint for the prototype. Upstream measured 13 MB in the same session.
- Both processes exposed 11 threads because AppKit, Core Animation and libdispatch create framework threads. The code creates no permanent worker of its own, so the product requirement must not claim a literally single-threaded process.

## Decisions to carry forward

1. Keep one main-thread `tao` event loop with a 2-second `WaitUntil` wake.
2. Reuse the retained two-line renderer approach and explicit autorelease pool.
3. Implement a dedicated macOS memory sampler instead of exposing sysinfo memory semantics.
4. Color the RAM value from `kern.memorystatus_vm_pressure_level`, independent of its numeric percentage.
5. Specify “no app-owned permanent workers in idle”, not “one process thread”.
6. Re-measure the production bundle over a long soak; these prototype numbers are directional, not a release benchmark.
