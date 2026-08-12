# Statlet production-bundle soak

- Started: 2026-08-12T22:35:39Z
- Bundle: Statlet 1.0.0; executable SHA-256 `c4f91b5a57c659248b71fcb0fa89dc50f21ecdc8b43ab67279cd596d4dc9fbe5`
- Signature: adhoc; flags=0x10002(adhoc,runtime)
- Host: Mac16,1, Apple M4, arm64, macOS 26.5.2 (25F84)
- Scenario: Idle menu-bar sampling; Mole integration disabled; no UI interaction during samples
- Preferences schema: v2 (verified in preferences.json)
- Mole integration: disabled (verified in v2 preferences.json)
- Metric refresh interval: 2 seconds (verified in v2 preferences.json)
- Requested duration: 1800 seconds; observed duration: 1807 seconds
- Warm-up excluded from samples: 10 seconds
- Requested sampling interval: 10 seconds
- Samples: 163
- RSS: 118480 KiB initial, 38384 KiB final, 26832–118480 KiB observed
- RSS growth: -80096 KiB; peak above initial: 0 KiB; observed range: 91648 KiB
- Physical footprint: 50496472 bytes initial, 48595928 bytes final, 109659072 bytes end-snapshot peak
- Mean sampled CPU: 0.325767%
- Idle-wakeup delta: 0 (0.000/s; macOS top IDLEW counter)
- Context-switch delta: 21641 (11.98/s; secondary scheduling context)
- Detailed physical-footprint snapshots: `footprint-start.txt` and `footprint-end.txt`

The run passes the bounded-growth guard when final RSS growth stays below 10 MiB, peak RSS stays below 20 MiB above the first post-warm-up sample, and mean sampled CPU stays below 1%. The full observed range remains informational because releasing memory increases that range without indicating a leak.
