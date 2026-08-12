# Statlet v1 production-bundle soak

- Started: 2026-08-12T00:30:31Z
- Bundle: Statlet 1.0.0; executable SHA-256 `d41bdcf80b9e81c54f62d1945d984ba0c848b9019f7fd2071a3c280bce6fb9ab`
- Signature: adhoc; flags=0x10002(adhoc,runtime)
- Host: Mac16,1, Apple M4, arm64, macOS 26.5.2 (25F84)
- Scenario: Idle menu-bar sampling; Mole integration disabled; no UI interaction during samples
- Mole integration: disabled (verified in preferences.json)
- Requested duration: 1800 seconds; observed duration: 1810 seconds
- Warm-up excluded from samples: 10 seconds
- Requested sampling interval: 10 seconds
- Samples: 169
- RSS: 55088 KiB initial, 31824 KiB final, 31136–55088 KiB observed
- RSS growth: -23264 KiB; peak above initial: 0 KiB; observed range: 23952 KiB
- Physical footprint: 20595528 bytes initial, 19989320 bytes final, 20841288 bytes end-snapshot peak
- Mean sampled CPU: 0.122485%
- Idle-wakeup delta: 0 (0.000/s; macOS top IDLEW counter)
- Context-switch delta: 20856 (11.52/s; secondary scheduling context)
- Detailed physical-footprint snapshots: footprint-start.txt and footprint-end.txt
- Raw samples: samples.csv

The run passes the v1 bounded-growth guard when final RSS growth stays below 10 MiB, peak RSS stays below 20 MiB above the first post-warm-up sample, and mean sampled CPU stays below 1%. The full observed range remains informational because releasing memory increases that range without indicating a leak.
