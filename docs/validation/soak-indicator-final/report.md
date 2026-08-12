# Statlet production-bundle soak

- Started: 2026-08-12T21:59:19Z
- Bundle: Statlet 1.0.0; executable SHA-256 `f7cb586f9910e7c49f091eb177598adbd80be35967d03e8759c79989a14c5f90`
- Signature: adhoc; flags=0x10002(adhoc,runtime)
- Host: Mac16,1, Apple M4, arm64, macOS 26.5.2 (25F84)
- Scenario: Idle menu-bar sampling; Mole integration disabled; no UI interaction during samples
- Preferences schema: v1 (verified in preferences.json)
- Mole integration: disabled (verified in v1 preferences.json)
- Metric refresh interval: 2 seconds (v1 migration default)
- Requested duration: 1800 seconds; observed duration: 1806 seconds
- Warm-up excluded from samples: 10 seconds
- Requested sampling interval: 10 seconds
- Samples: 162
- RSS: 114480 KiB initial, 30256 KiB final, 28256–114480 KiB observed
- RSS growth: -84224 KiB; peak above initial: 0 KiB; observed range: 86224 KiB
- Physical footprint: 49775576 bytes initial, 45351896 bytes final, 110035904 bytes end-snapshot peak
- Mean sampled CPU: 0.253704%
- Idle-wakeup delta: 0 (0.000/s; macOS top IDLEW counter)
- Context-switch delta: 39452 (21.84/s; secondary scheduling context)
- Detailed physical-footprint snapshots: `footprint-start.txt` and `footprint-end.txt`

The run passes the bounded-growth guard when final RSS growth stays below 10 MiB, peak RSS stays below 20 MiB above the first post-warm-up sample, and mean sampled CPU stays below 1%. The full observed range remains informational because releasing memory increases that range without indicating a leak.
