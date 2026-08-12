# Indicator Customization Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add curated, immediately applied customization for Statlet's CPU/RAM colors, labels, typography, and 1–60 second refresh interval while preserving accessibility, stable width, disk timing, and the v1 performance model.

**Architecture:** Keep validated preferences and indicator composition in the platform-independent library; keep AppKit font resolution, drawing, previews, and controls in focused macOS adapters. The reducer remains the single source of truth, while the runtime combines independent metric and disk deadlines into one `ControlFlow::WaitUntil` and redraws the real indicator plus visible previews from the latest snapshot without extra polling.

**Tech Stack:** Rust 1.89, AppKit through `objc2`/`objc2-app-kit` 0.3.2, `tao` event loop, `tray-icon`, `serde_json`, native macOS accessibility APIs, Rust integration tests.

## Global Constraints

- Implement from the commit containing this approved plan (whose design parent is `14ecc8746d7b326cd03f70534395b96bacd7def5`) in an isolated worktree created with `superpowers:using-git-worktrees`; do not implement directly on the current `main` checkout.
- Read `CONTEXT.md`, ADR 0001, and `docs/superpowers/specs/2026-08-12-indicator-customization-design.md` before the first code change.
- Preserve one status item with CPU above RAM; do not add reordering, per-metric intervals, custom label text, alpha, presets, graphs, processes, or new permanent metrics.
- Preserve CPU, RAM, memory-pressure, disk, notification, history, and Mole semantics exactly.
- Defaults must reproduce v1: dynamic CPU/RAM, visible neutral labels, system monospaced 12 pt medium, and 2-second metric refresh.
- Accept only whole font sizes `9..=14` and whole metric intervals `1..=60` seconds.
- Persist colors as uppercase sRGB `#RRGGBB`; never persist alpha, undo state, save status, resolved fallback fonts, or transient invalid text.
- Keep the macOS 14.0 arm64 floor, Rust 1.89, direct distribution contract, and existing dependencies; enable existing `objc2` features but add no crate unless a separately reviewed blocker proves it necessary.
- Preserve one main event loop, one combined `WaitUntil`, no new permanent worker, no preview timer, retained reusable renderer state, and one autorelease pool per sampling/redraw cycle.
- All shell commands in this repository must be prefixed with `rtk`.
- Do not commit `.superpowers/`; it contains local brainstorming artifacts.

## File Structure

### New library files

- `src/indicator_preferences.rs`: validated value objects, defaults, preference mutations, and reset groups.
- `src/indicator.rs`: AppKit-independent status content, scene composition, semantic/fixed segment colors, stable-layout calculation, and diagnostics.
- `src/metrics_schedule.rs`: configurable metric schedule independent of the disk schedule.
- `src/preferences_view.rs`: pure transient UI models for hex drafts, disclosure state, font filtering, interval drafts, and warnings.

### New macOS files

- `src/macos/fonts.rs`: installed-family catalog, semantic weight resolution, fallback, and cache invalidation.
- `src/macos/environment.rs`: appearance/accessibility/font/screen observers that only emit runtime events.
- `src/macos/windows/common.rs`: shared window, label, group, and accessibility helpers.
- `src/macos/windows/history.rs`: mechanically extracted history window.
- `src/macos/windows/free_space.rs`: mechanically extracted free-space window.
- `src/macos/windows/preferences/mod.rs`: preferences window shell, area selector, lifecycle, footer, and page routing.
- `src/macos/windows/preferences/indicator.rs`: CPU/RAM, labels, typography, and update groups.
- `src/macos/windows/preferences/color_editor.rs`: reusable `NSColorWell` + hex + variants editor.
- `src/macos/windows/preferences/font_picker.rs`: searchable, accessible font-family sheet with samples.
- `src/macos/windows/preferences/preview.rs`: light/dark images and textual warnings.

### Existing files with focused changes

- `src/lib.rs`: export new library modules.
- `src/core.rs`: reducer state/events/effects, numeric status content, reset/undo, save status.
- `src/preferences.rs`: explicit v1/v2 DTO dispatch and v2 atomic persistence.
- `src/disk.rs`: expose the disk schedule's remaining deadline.
- `src/main.rs`: non-`Copy` effects, save-result feedback, independent schedules, redraw orchestration.
- `src/macos/mod.rs`: export new adapters and runtime events.
- `src/macos/renderer.rs`: mutable three-slot renderer shared by real status and previews.
- `src/macos/windows.rs`: shrink to `WindowManager` and child modules.
- `Cargo.toml`: enable the exact AppKit/Foundation features used by the native UI.
- `docs/product/v1.md`, `README.md`, `docs/validation/accessibility-lifecycle.md`: accurately document the next-version behavior and residual manual gates.

---

### Task 1: Validated indicator preference model

**Files:**
- Create: `src/indicator_preferences.rs`
- Modify: `src/lib.rs`
- Test: `tests/indicator_preferences.rs`

**Interfaces:**
- Produces: `SrgbColor`, `IndicatorAppearance`, `MetricKind`, `MetricColorMode`, `FixedColorPreferences`, `MetricColorPreferences`, `LabelColorMode`, `LabelPreferences`, `FontFamilyPreference`, `FontSize`, `FontWeight`, `TypographyPreferences`, `MetricsRefreshInterval`, `IndicatorPreferences`, and `IndicatorPreferenceGroup`.
- Defaults: inactive fixed seeds are CPU `#34C759`, RAM `#0A84FF`, and custom labels `#8E8E93`; they are not rendered while dynamic/neutral defaults are active.

- [ ] **Step 1: Write failing value-object tests**

```rust
use statlet::indicator_preferences::{
    FontSize, IndicatorPreferences, MetricsRefreshInterval, SrgbColor,
};

#[test]
fn hex_accepts_hash_or_bare_rgb_and_normalizes_uppercase() {
    assert_eq!(SrgbColor::parse_hex("#0a84ff").unwrap().to_hex(), "#0A84FF");
    assert_eq!(SrgbColor::parse_hex("34c759").unwrap().to_hex(), "#34C759");
}

#[test]
fn hex_rejects_alpha_short_and_non_hex_values() {
    for value in ["#0A84FFFF", "#FFF", "0A84F", "#GG84FF"] {
        assert!(SrgbColor::parse_hex(value).is_err(), "accepted {value}");
    }
}

#[test]
fn bounded_values_accept_only_the_approved_ranges() {
    assert!(FontSize::try_from(8).is_err());
    assert_eq!(FontSize::try_from(9).unwrap().points(), 9);
    assert_eq!(FontSize::try_from(14).unwrap().points(), 14);
    assert!(FontSize::try_from(15).is_err());
    assert!(MetricsRefreshInterval::try_from(0).is_err());
    assert_eq!(MetricsRefreshInterval::try_from(60).unwrap().seconds(), 60);
    assert!(MetricsRefreshInterval::try_from(61).is_err());
}

#[test]
fn defaults_exactly_match_the_v1_indicator() {
    let value = IndicatorPreferences::default();
    assert!(value.labels.visible);
    assert_eq!(value.typography.size.points(), 12);
    assert_eq!(value.refresh_interval.seconds(), 2);
    assert!(value.cpu_color.is_dynamic());
    assert!(value.ram_color.is_dynamic());
}
```

- [ ] **Step 2: Run the test and verify the module is missing**

Run: `rtk cargo test --test indicator_preferences`

Expected: FAIL because `statlet::indicator_preferences` is not exported.

- [ ] **Step 3: Implement the validated types and exact defaults**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct SrgbColor([u8; 3]);

impl SrgbColor {
    pub fn parse_hex(input: &str) -> Result<Self, InvalidHexColor>;
    pub fn to_hex(self) -> String;
    pub const fn components(self) -> [u8; 3];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum IndicatorAppearance { Light, Dark }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricKind { Cpu, Ram }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MetricColorMode { Dynamic, Fixed }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AppearanceColors { pub light: SrgbColor, pub dark: SrgbColor }

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FixedColorPreferences {
    pub shared: SrgbColor,
    pub use_appearance_variants: bool,
    pub variants: Option<AppearanceColors>,
}

impl FixedColorPreferences {
    pub fn set_variants_enabled(&mut self, enabled: bool);
    pub fn color_for(self, appearance: IndicatorAppearance) -> SrgbColor;
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FontFamilyPreference { SystemMonospaced, Named(String) }

impl FontFamilyPreference {
    pub fn named(value: impl Into<String>) -> Result<Self, InvalidFontFamily>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FontSize(u8);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MetricsRefreshInterval(u8);
```

Add `pub mod indicator_preferences;` to `src/lib.rs`. Keep named-font validation to non-empty trimmed names; installation is an AppKit adapter concern.

- [ ] **Step 4: Add variant-retention and group-reset tests**

```rust
#[test]
fn disabling_variants_preserves_them_for_reenable_but_group_reset_removes_them() {
    let mut value = IndicatorPreferences::default();
    value.cpu_color.fixed.set_variants_enabled(true);
    let remembered = value.cpu_color.fixed.variants.unwrap();
    value.cpu_color.fixed.set_variants_enabled(false);
    assert_eq!(value.cpu_color.fixed.variants, Some(remembered));
    value.cpu_color.fixed.set_variants_enabled(true);
    assert_eq!(value.cpu_color.fixed.variants, Some(remembered));
    value.reset(IndicatorPreferenceGroup::CpuAndRam);
    assert_eq!(value.cpu_color, IndicatorPreferences::default().cpu_color);
}
```

- [ ] **Step 5: Run focused tests and lint**

Run: `rtk cargo test --test indicator_preferences && rtk cargo clippy --all-targets -- -D warnings`

Expected: PASS.

- [ ] **Step 6: Commit the preference model**

```bash
rtk git add src/lib.rs src/indicator_preferences.rs tests/indicator_preferences.rs
rtk git commit -m "feat: model indicator preferences"
```

### Task 2: Versioned v1-to-v2 preference persistence

**Files:**
- Modify: `src/core.rs`
- Modify: `src/preferences.rs`
- Modify: `tests/preferences_store.rs`
- Modify: `tests/preferences_flow.rs`
- Modify: `tests/disk_pressure.rs`
- Modify: `tests/history_flow.rs`
- Modify: `tests/lifecycle_accessibility.rs`
- Modify: `tests/notification_mole_flow.rs`

**Interfaces:**
- Consumes: `IndicatorPreferences` from Task 1.
- Produces: root `Preferences { mole_integration_enabled, warning_threshold, indicator }`, explicit `StoredPreferencesV1` and `StoredPreferencesV2`, and v2 JSON.

- [ ] **Step 1: Add failing migration and v2 round-trip tests**

```rust
#[test]
fn version_one_migrates_disk_values_and_defaults_the_indicator() {
    fs::write(&path, r#"{"version":1,"moleIntegrationEnabled":true,"warningThreshold":95}"#).unwrap();
    let loaded = store.load();
    assert!(loaded.mole_integration_enabled);
    assert_eq!(loaded.warning_threshold.get(), 95);
    assert_eq!(loaded.indicator, IndicatorPreferences::default());
}

#[test]
fn version_two_round_trip_preserves_nested_indicator_preferences() {
    let mut expected = Preferences::default();
    expected.indicator.refresh_interval = MetricsRefreshInterval::try_from(17).unwrap();
    expected.indicator.typography.family = FontFamilyPreference::named("Avenir Next").unwrap();
    store.save(expected.clone()).unwrap();
    assert_eq!(store.load(), expected);
    assert_eq!(serde_json::from_str::<Value>(&fs::read_to_string(path).unwrap()).unwrap()["version"], 2);
}
```

- [ ] **Step 2: Verify the current v1 loader fails the new expectations**

Run: `rtk cargo test --test preferences_store`

Expected: FAIL because v1 is rejected and `Preferences` has no indicator block.

- [ ] **Step 3: Make root preferences cloneable and update literals safely**

```rust
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Preferences {
    pub mole_integration_enabled: bool,
    pub warning_threshold: WarningThreshold,
    pub indicator: IndicatorPreferences,
}

impl Default for Preferences {
    fn default() -> Self {
        Self {
            mole_integration_enabled: false,
            warning_threshold: WarningThreshold::default(),
            indicator: IndicatorPreferences::default(),
        }
    }
}
```

Update every existing literal to use `..Preferences::default()` when it does not intentionally customize the indicator. Replace moves that previously relied on `Copy` with explicit `.clone()`.

- [ ] **Step 4: Implement explicit version-envelope dispatch**

```rust
#[derive(Deserialize)]
struct StoredVersion { version: u8 }

fn decode(bytes: &[u8]) -> Option<Preferences> {
    match serde_json::from_slice::<StoredVersion>(bytes).ok()?.version {
        1 => serde_json::from_slice::<StoredPreferencesV1>(bytes).ok()?.into_preferences(),
        2 => serde_json::from_slice::<StoredPreferencesV2>(bytes).ok()?.into_preferences(),
        _ => None,
    }
}
```

Represent enums in camelCase, colors as `#RRGGBB`, the system font as `"systemMonospaced"`, and a named family as `{ "named": "Avenir Next" }`. Validate every DTO field through Task 1 types before producing domain preferences.

- [ ] **Step 5: Add corruption tests for nested v2 values**

Test invalid color alpha, size 8/15, interval 0/61, blank named family, version 3, and missing `indicator`. Each must load `Preferences::default()` rather than partially applying data.

- [ ] **Step 6: Run persistence and existing preference tests**

Run: `rtk cargo test --test preferences_store --test preferences_flow --test disk_pressure --test history_flow --test lifecycle_accessibility --test notification_mole_flow`

Expected: PASS with existing Mole/disk behavior unchanged.

- [ ] **Step 7: Commit migration and root preference changes**

```bash
rtk git add src/core.rs src/preferences.rs tests
rtk git commit -m "feat: migrate indicator preferences to v2"
```

### Task 3: AppKit-independent indicator composition and layout

**Files:**
- Create: `src/indicator.rs`
- Modify: `src/lib.rs`
- Modify: `src/core.rs`
- Modify: `tests/status_presentation.rs`
- Create: `tests/indicator_presentation.rs`
- Create: `tests/indicator_layout.rs`

**Interfaces:**
- Consumes: indicator preference types from Task 1.
- Produces: `StatusContent`, `IndicatorRun`, `IndicatorScene`, `SegmentColor`, `SemanticColor`, `compose_indicator`, `TextMeasurer`, `StableLayout`, and `LayoutDiagnostics`.

- [ ] **Step 1: Protect numeric metrics and accessibility before changing presentation**

```rust
#[test]
fn status_content_preserves_rounded_metrics_and_complete_accessibility() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::MetricsSample(SystemSnapshot {
        cpu_percent: 39.4,
        ram_percent: 72.6,
        memory_pressure: MemoryPressure::Normal,
    }));
    assert_eq!(app.state().status.cpu.percent, 39);
    assert_eq!(app.state().status.ram.percent, 73);
    assert_eq!(app.state().status.accessibility_label,
        "CPU 39%, RAM 73%, pressão de memória normal");
}
```

- [ ] **Step 2: Replace preformatted values with numeric `StatusContent`**

```rust
pub struct MetricContent {
    pub label: &'static str,
    pub percent: u8,
    pub severity: MetricSeverity,
}

pub struct StatusContent {
    pub cpu: MetricContent,
    pub ram: MetricContent,
    pub disk_badge: Option<DiskBadge>,
    pub accessibility_label: String,
}
```

Keep rounding, thresholds, memory-pressure descriptions, and disk accessibility wording in `core.rs`; move only visual formatting into `indicator.rs`.

- [ ] **Step 3: Write failing composition tests for all color and label modes**

```rust
fn status() -> StatusContent {
    StatusContent {
        cpu: MetricContent { label: "C", percent: 42, severity: MetricSeverity::Warning },
        ram: MetricContent { label: "R", percent: 68, severity: MetricSeverity::Good },
        disk_badge: None,
        accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".into(),
    }
}

fn critical_status() -> StatusContent {
    StatusContent {
        cpu: MetricContent { severity: MetricSeverity::Critical, ..status().cpu },
        ..status()
    }
}

#[test]
fn hidden_labels_remove_text_but_not_accessibility() {
    let mut preferences = IndicatorPreferences::default();
    preferences.labels.visible = false;
    let scene = compose_indicator(&status(), &preferences, IndicatorAppearance::Light);
    assert_eq!(scene.top.iter().map(|run| run.text.as_str()).collect::<String>(), "42%");
    assert_eq!(scene.bottom.iter().map(|run| run.text.as_str()).collect::<String>(), "68%");
    assert!(scene.accessibility_label.starts_with("CPU 42%, RAM 68%"));
}

#[test]
fn fixed_cpu_and_dark_variant_ignore_severity() {
    let mut preferences = IndicatorPreferences::default();
    preferences.cpu_color.mode = MetricColorMode::Fixed;
    preferences.cpu_color.fixed.set_variants_enabled(true);
    preferences.cpu_color.fixed.variants.as_mut().unwrap().dark = SrgbColor::parse_hex("#AF52DE").unwrap();
    let scene = compose_indicator(&critical_status(), &preferences, IndicatorAppearance::Dark);
    assert_eq!(scene.top.last().unwrap().color, SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap()));
}
```

- [ ] **Step 4: Implement deterministic scene composition**

```rust
pub enum SemanticColor { Neutral, Good, Warning, Critical, DiskWarning, DiskError }
pub enum SegmentColor { Semantic(SemanticColor), Srgb(SrgbColor) }
pub struct IndicatorRun { pub text: String, pub color: SegmentColor }
pub struct IndicatorScene {
    pub top: Vec<IndicatorRun>,
    pub bottom: Vec<IndicatorRun>,
    pub disk_badge: Option<IndicatorRun>,
    pub accessibility_label: String,
}

pub fn compose_indicator(
    status: &StatusContent,
    preferences: &IndicatorPreferences,
    appearance: IndicatorAppearance,
) -> IndicatorScene;
```

Use `"C "`/`"R "` label runs and unpadded `"42%"` values. `MatchesMetric` copies each metric value color. Custom labels resolve through their own fixed preferences. Badge symbols remain ` !` and ` ×`.

- [ ] **Step 5: Write failing layout tests with a fake proportional measurer**

```rust
struct FakeMeasurer;
impl TextMeasurer for FakeMeasurer {
    fn width(&self, text: &str) -> f64 {
        text.chars().map(|c| if c == '1' { 3.0 } else { 7.0 }).sum()
    }
    fn content_height(&self) -> f64 { 18.0 }
}

#[test]
fn stable_layout_uses_the_widest_value_from_zero_through_one_hundred() {
    let measurer = FakeMeasurer;
    let layout = measure_stable_layout(&FakeMeasurer, true, 40.0);
    assert!(layout.cpu_width >= measurer.width("C 100%"));
    assert!(layout.ram_width >= measurer.width("R 100%"));
}

#[test]
fn badge_width_is_added_only_while_the_badge_exists() {
    let base = measure_stable_layout(&FakeMeasurer, true, 40.0);
    assert_eq!(base.width_for_badge(None), base.base_width());
    assert!(base.width_for_badge(Some(" !")) > base.base_width());
}
```

- [ ] **Step 6: Implement stable measurement and warning thresholds**

```rust
pub trait TextMeasurer {
    fn width(&self, text: &str) -> f64;
    fn content_height(&self) -> f64;
}

pub struct LayoutDiagnostics {
    pub exceeds_menu_bar_height: bool,
    pub exceeds_curated_width: bool,
}

pub fn measure_stable_layout(
    measurer: &impl TextMeasurer,
    labels_visible: bool,
    default_width: f64,
) -> StableLayout;
```

Measure both metric prefixes across all `0..=100`; warn above 22 pt content height or above twice `default_width`. Do not include badge width in the cached base width.

- [ ] **Step 7: Run composition/layout regressions**

Run: `rtk cargo test --test status_presentation --test indicator_presentation --test indicator_layout --test memory_metrics`

Expected: PASS.

- [ ] **Step 8: Commit pure presentation and layout**

```bash
rtk git add src/lib.rs src/core.rs src/indicator.rs tests/status_presentation.rs tests/indicator_presentation.rs tests/indicator_layout.rs
rtk git commit -m "feat: compose customizable indicator scenes"
```

### Task 4: Reducer events, immediate redraw, save feedback, reset, and undo

**Files:**
- Modify: `src/core.rs`
- Create: `tests/indicator_preferences_flow.rs`
- Modify: `tests/preferences_flow.rs`

**Interfaces:**
- Consumes: Task 1 preferences and Task 3 status content.
- Produces: `IndicatorPreferenceChange`, `PreferencesSaveStatus`, `PreferencesSaveResult`, new `AppEvent` variants, and `AppEffect::{RedrawIndicator, SetMetricsSamplingInterval}`.

- [ ] **Step 1: Write failing reducer tests for a visual change and interval change**

```rust
#[test]
fn visual_change_redraws_then_saves_the_complete_document() {
    let mut app = StatletCore::new();
    let color = SrgbColor::parse_hex("#AF52DE").unwrap();
    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetMetricSharedColor { metric: MetricKind::Cpu, color },
    ));
    assert_eq!(effects[0], AppEffect::RedrawIndicator);
    assert_eq!(effects[1], AppEffect::SavePreferences(app.state().preferences.clone()));
}

#[test]
fn interval_change_reschedules_without_collecting() {
    let mut app = StatletCore::new();
    let interval = MetricsRefreshInterval::try_from(17).unwrap();
    let effects = app.handle(AppEvent::UpdateIndicator(
        IndicatorPreferenceChange::SetRefreshInterval(interval),
    ));
    assert_eq!(effects, vec![
        AppEffect::SetMetricsSamplingInterval(interval),
        AppEffect::RedrawIndicator,
        AppEffect::SavePreferences(app.state().preferences.clone()),
    ]);
}
```

- [ ] **Step 2: Add typed change, reset group, reset global, and lifecycle events**

```rust
pub enum AppEvent {
    UpdateIndicator(IndicatorPreferenceChange),
    ResetIndicatorGroup(IndicatorPreferenceGroup),
    ResetIndicatorConfirmed,
    UndoIndicatorReset,
    PreferencesWindowClosed,
    RetrySavePreferences,
    PreferencesSaveFinished(PreferencesSaveResult),
    // existing variants remain
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum AppEffect {
    RedrawIndicator,
    SetMetricsSamplingInterval(MetricsRefreshInterval),
    SavePreferences(Preferences),
    // existing effects remain
}
```

`AppEvent` and `AppEffect` must be `Clone`, not `Copy`. No-op updates emit no effect.

- [ ] **Step 3: Add failing reset/undo tests**

```rust
fn customized_app(mole_enabled: bool) -> StatletCore {
    let mut preferences = Preferences::default();
    preferences.mole_integration_enabled = mole_enabled;
    preferences.indicator.typography.size = FontSize::try_from(14).unwrap();
    StatletCore::with_preferences(preferences).0
}

#[test]
fn global_reset_keeps_disk_preferences_and_undo_replaces_later_indicator_edits() {
    let mut app = customized_app(true);
    let before = app.state().preferences.indicator.clone();
    app.handle(AppEvent::ResetIndicatorConfirmed);
    assert_eq!(app.state().preferences.indicator, IndicatorPreferences::default());
    assert!(app.state().can_undo_indicator_reset);
    app.handle(AppEvent::UpdateIndicator(IndicatorPreferenceChange::SetFontSize(FontSize::try_from(14).unwrap())));
    app.handle(AppEvent::UndoIndicatorReset);
    assert_eq!(app.state().preferences.indicator, before);
    assert!(app.state().preferences.mole_integration_enabled);
    assert!(!app.state().can_undo_indicator_reset);
}

#[test]
fn closing_preferences_discards_only_the_transient_undo_snapshot() {
    let mut app = customized_app(false);
    app.handle(AppEvent::ResetIndicatorConfirmed);
    app.handle(AppEvent::PreferencesWindowClosed);
    assert!(app.handle(AppEvent::UndoIndicatorReset).is_empty());
}
```

- [ ] **Step 4: Implement reducer-owned one-level undo**

Add `indicator_reset_undo: Option<IndicatorPreferences>` to `StatletCore` and `can_undo_indicator_reset: bool` plus `preferences_save_status` to `AppState`. Group reset does not confirm; global reset replaces the previous snapshot. Only a changed interval emits `SetMetricsSamplingInterval`.

- [ ] **Step 5: Add save failure/retry tests**

```rust
fn change_color(hex: &str) -> IndicatorPreferenceChange {
    IndicatorPreferenceChange::SetMetricSharedColor {
        metric: MetricKind::Cpu,
        color: SrgbColor::parse_hex(hex).unwrap(),
    }
}

fn change_interval(seconds: u8) -> IndicatorPreferenceChange {
    IndicatorPreferenceChange::SetRefreshInterval(
        MetricsRefreshInterval::try_from(seconds).unwrap(),
    )
}

#[test]
fn save_failure_keeps_session_state_and_retry_uses_the_latest_document() {
    let mut app = StatletCore::new();
    app.handle(AppEvent::UpdateIndicator(change_color("#AF52DE")));
    app.handle(AppEvent::PreferencesSaveFinished(PreferencesSaveResult::Failed));
    assert_eq!(app.state().preferences_save_status, PreferencesSaveStatus::Failed);
    app.handle(AppEvent::UpdateIndicator(change_interval(9)));
    assert_eq!(app.handle(AppEvent::RetrySavePreferences), vec![
        AppEffect::SavePreferences(app.state().preferences.clone())
    ]);
    app.handle(AppEvent::PreferencesSaveFinished(PreferencesSaveResult::Saved));
    assert_eq!(app.state().preferences_save_status, PreferencesSaveStatus::Saved);
}
```

- [ ] **Step 6: Run reducer suites**

Run: `rtk cargo test --test indicator_preferences_flow --test preferences_flow --test notification_mole_flow --test history_flow`

Expected: PASS.

- [ ] **Step 7: Commit reducer behavior**

```bash
rtk git add src/core.rs tests/indicator_preferences_flow.rs tests/preferences_flow.rs
rtk git commit -m "feat: handle indicator preference changes"
```

### Task 5: Independent metric and disk deadlines in one event loop

**Files:**
- Create: `src/metrics_schedule.rs`
- Modify: `src/lib.rs`
- Modify: `src/disk.rs`
- Modify: `src/main.rs`
- Create: `tests/metrics_sampling_schedule.rs`
- Modify: `tests/disk_sampling_schedule.rs`

**Interfaces:**
- Consumes: `MetricsRefreshInterval`, `AppEffect::SetMetricsSamplingInterval`.
- Produces: `MetricsSamplingSchedule::{new_due_now,reschedule,take_due,remaining}`, `DiskSamplingSchedule::remaining`, `RuntimeSamplers::{poll_due,reschedule_metrics,next_wakeup_in}`.

- [ ] **Step 1: Write failing metric-schedule tests**

```rust
#[test]
fn schedule_is_due_now_then_uses_the_default_two_seconds() {
    let now = seconds(10);
    let mut schedule = MetricsSamplingSchedule::new_due_now(now, MetricsRefreshInterval::default());
    assert!(schedule.take_due(now));
    assert_eq!(schedule.remaining(now), seconds(2));
}

#[test]
fn reschedule_waits_the_new_interval_without_immediate_sample() {
    let mut schedule = MetricsSamplingSchedule::new_due_now(seconds(0), interval(2));
    assert!(schedule.take_due(seconds(0)));
    schedule.reschedule(seconds(1), interval(60));
    assert!(!schedule.take_due(seconds(1)));
    assert_eq!(schedule.remaining(seconds(1)), seconds(60));
}

#[test]
fn delayed_wakeup_samples_once_without_a_catch_up_burst() {
    let mut schedule = MetricsSamplingSchedule::new_due_now(seconds(0), interval(2));
    assert!(schedule.take_due(seconds(0)));
    assert!(schedule.take_due(seconds(120)));
    assert!(!schedule.take_due(seconds(120)));
}
```

- [ ] **Step 2: Implement the pure metric schedule**

```rust
pub struct MetricsSamplingSchedule {
    interval: Duration,
    next_due: Duration,
}

impl MetricsSamplingSchedule {
    pub fn new_due_now(now: Duration, interval: MetricsRefreshInterval) -> Self;
    pub fn reschedule(&mut self, now: Duration, interval: MetricsRefreshInterval);
    pub fn take_due(&mut self, now: Duration) -> bool;
    pub fn remaining(&self, now: Duration) -> Duration;
}
```

After a due or delayed wake, set `next_due = now + interval`; never loop to catch up. Construct `RuntimeSamplers` with the interval from the already-loaded root preferences so there is no second default source in `main.rs`.

- [ ] **Step 3: Extend disk schedule with a tested remaining deadline**

```rust
#[test]
fn disabled_disk_has_no_deadline_and_enabled_disk_reports_remaining_time() {
    let mut schedule = DiskSamplingSchedule::new();
    assert_eq!(schedule.remaining(seconds(0)), None);
    schedule.set_enabled(true, seconds(0));
    assert_eq!(schedule.remaining(seconds(0)), Some(Duration::ZERO));
}
```

- [ ] **Step 4: Refactor runtime polling and combined deadline**

```rust
impl RuntimeSamplers {
    fn poll_due(&mut self, core: &mut StatletCore) -> Vec<AppEffect>;
    fn reschedule_metrics(&mut self, interval: MetricsRefreshInterval);
    fn next_wakeup_in(&self) -> Duration {
        self.disk_schedule
            .remaining(self.clock.now())
            .map_or_else(|| self.metrics_schedule.remaining(self.clock.now()), |disk| {
                disk.min(self.metrics_schedule.remaining(self.clock.now()))
            })
    }
}
```

Remove `METRICS_REFRESH`. On startup construct the metric schedule from loaded preferences. Both `Init` and `ResumeTimeReached` call `poll_due`, then set exactly one `WaitUntil(Instant::now() + next_wakeup_in())`. `SetMetricsSamplingInterval` reschedules metrics only and immediately recomputes `WaitUntil`; it never samples and never changes disk timing.

- [ ] **Step 5: Convert effect processing from `.copied()` to owned clones**

Use `effects.iter().cloned().collect::<VecDeque<_>>()`. For `SavePreferences`, call `save(preferences.clone())`, log the concrete `io::Error`, and enqueue `PreferencesSaveFinished(Saved|Failed)` back through the reducer.

- [ ] **Step 6: Run schedule and lifecycle tests**

Run: `rtk cargo test --test metrics_sampling_schedule --test disk_sampling_schedule --test lifecycle_accessibility --test preferences_flow`

Expected: PASS; changing metric interval does not move the disk deadline.

- [ ] **Step 7: Commit schedule/runtime changes**

```bash
rtk git add src/lib.rs src/metrics_schedule.rs src/disk.rs src/main.rs tests/metrics_sampling_schedule.rs tests/disk_sampling_schedule.rs
rtk git commit -m "feat: schedule configurable metric refresh"
```

### Task 6: Font resolution and the shared three-slot renderer

**Files:**
- Create: `src/macos/fonts.rs`
- Modify: `src/macos/mod.rs`
- Modify: `src/macos/renderer.rs`
- Modify: `Cargo.toml`
- Test: unit tests in `src/macos/fonts.rs` and `src/macos/renderer.rs`

**Interfaces:**
- Consumes: `IndicatorScene`, `StableLayout`, `TypographyPreferences`, `IndicatorAppearance`.
- Produces: `FontCatalog`, `FontResolution`, `RenderSlot`, `RenderOutput`, `PreviewImages`, and mutable `Renderer` APIs used by Task 7 and the preferences window.

- [ ] **Step 1: Enable only the required native features for renderer/font work**

Add these `objc2-app-kit` features in `Cargo.toml`: `NSAppearance`, `NSColorSpace`, `NSFontManager`, and `NSWorkspace`. Add `NSNotification` and `NSKeyValueObserving` to `objc2-foundation`. Do not add a crate.

- [ ] **Step 2: Write failing font-resolution tests on the main thread**

```rust
#[test]
fn missing_named_family_uses_system_monospaced_without_rewriting_the_request() {
    let catalog = FontCatalog::new(MainThreadMarker::new().unwrap());
    let requested = TypographyPreferences {
        family: FontFamilyPreference::named("Statlet Definitely Missing").unwrap(),
        ..TypographyPreferences::default()
    };
    let resolved = catalog.resolve(&requested);
    assert!(resolved.used_fallback);
    assert_eq!(resolved.requested_family, requested.family);
}
```

Also test the system default, installed named family, semantic weight mapping, catalog filter ordering, invalidation, and recovery when a fake catalog begins returning the requested family.

- [ ] **Step 3: Implement cached font catalog and resolution**

```rust
pub struct FontResolution {
    pub font: Retained<NSFont>,
    pub requested_family: FontFamilyPreference,
    pub resolved_family: String,
    pub used_fallback: bool,
}

pub struct FontCatalog {
    manager: Retained<NSFontManager>,
    families: Vec<String>,
}

impl FontCatalog {
    pub fn new(mtm: MainThreadMarker) -> Self;
    pub fn families(&self) -> &[String];
    pub fn resolve(&self, preferences: &TypographyPreferences) -> FontResolution;
    pub fn refresh(&mut self);
}
```

Resolve named families with `fontWithFamily_traits_weight_size`; use the nearest AppKit face. Never mutate the persisted request on fallback.

- [ ] **Step 4: Write failing renderer-slot and cache tests**

```rust
fn scene(value_color: SegmentColor) -> IndicatorScene {
    IndicatorScene {
        top: vec![IndicatorRun { text: "C 42%".into(), color: value_color }],
        bottom: vec![IndicatorRun { text: "R 68%".into(), color: value_color }],
        disk_badge: None,
        accessibility_label: "CPU 42%, RAM 68%, pressão de memória normal".into(),
    }
}

fn aqua() -> Retained<NSAppearance> {
    unsafe { NSAppearance::appearanceNamed(NSAppearanceNameAqua) }.unwrap()
}

fn dark_aqua() -> Retained<NSAppearance> {
    unsafe { NSAppearance::appearanceNamed(NSAppearanceNameDarkAqua) }.unwrap()
}

#[test]
fn renderer_keeps_exactly_one_cache_entry_per_surface() {
    let mtm = MainThreadMarker::new().unwrap();
    let mut renderer = Renderer::new(mtm);
    let typography = TypographyPreferences::default();
    let semantic = scene(SegmentColor::Semantic(SemanticColor::Warning));
    let fixed = scene(SegmentColor::Srgb(SrgbColor::parse_hex("#AF52DE").unwrap()));
    renderer.render(RenderSlot::Status, &semantic, &typography, &aqua());
    renderer.render(RenderSlot::PreviewLight, &semantic, &typography, &aqua());
    renderer.render(RenderSlot::PreviewDark, &semantic, &typography, &dark_aqua());
    renderer.render(RenderSlot::Status, &fixed, &typography, &aqua());
    assert_eq!(renderer.cached_slot_count(), 3);
}
```

- [ ] **Step 5: Replace fixed attributes with mutable slot caches**

```rust
pub enum RenderSlot { Status, PreviewLight, PreviewDark }

pub struct RenderOutput {
    pub image: Retained<NSImage>,
    pub layout: LayoutDiagnostics,
    pub font: FontResolution,
}

pub struct PreviewImages {
    pub light: Retained<NSImage>,
    pub dark: Retained<NSImage>,
}

struct SlotCache {
    layout_key: LayoutKey,
    layout: StableLayout,
    paint_key: PaintKey,
    image: Retained<NSImage>,
}

impl Renderer {
    pub fn new(mtm: MainThreadMarker) -> Self;
    pub fn render(
        &mut self,
        slot: RenderSlot,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
        appearance: &NSAppearance,
    ) -> RenderOutput;
    pub fn apply_status(
        &mut self,
        button: &NSStatusBarButton,
        scene: &IndicatorScene,
        typography: &TypographyPreferences,
    ) -> LayoutDiagnostics;
}
```

Use `button.effectiveAppearance()` for the real status. Use Aqua/DarkAqua and `performAsCurrentDrawingAppearance` for previews. Convert `SegmentColor::Srgb` to `NSColor` in the sRGB color space. Use the same font-backed `TextMeasurer` for stable layout and drawing. Keep disk badge width transient.

`LayoutKey` contains resolved family, size, weight, and label visibility. `PaintKey` contains the `LayoutKey`, appearance, and scene colors. Reuse `StableLayout` when only paint changes so editing a color does not repeat the 202 width measurements; replace, rather than append to, each of the three `SlotCache` entries.

- [ ] **Step 6: Preserve the single accessible status item**

`apply_status` must set both `setAccessibilityLabel` and `setToolTip` from `IndicatorScene::accessibility_label`, independent of label visibility or colors.

- [ ] **Step 7: Run renderer, presentation, and package compilation checks**

Run: `rtk cargo test --bin statlet macos:: && rtk cargo test --test indicator_presentation --test indicator_layout && rtk cargo clippy --all-targets -- -D warnings`

Expected: PASS.

- [ ] **Step 8: Commit renderer and font adapter**

```bash
rtk git add Cargo.toml Cargo.lock src/macos/mod.rs src/macos/fonts.rs src/macos/renderer.rs
rtk git commit -m "feat: render custom indicator typography"
```

### Task 7: Visual-environment observation and redraw orchestration

**Files:**
- Create: `src/macos/environment.rs`
- Modify: `src/macos/mod.rs`
- Modify: `src/main.rs`
- Modify: `src/macos/windows.rs`
- Test: unit tests in `src/main.rs` and `src/macos/environment.rs`

**Interfaces:**
- Consumes: Task 6 renderer; core `RedrawIndicator`; current `AppState`.
- Produces: `VisualEnvironment`, `VisualEnvironmentObserver`, `RuntimeEvent::{VisualEnvironmentChanged,FontSetChanged,ScreenParametersChanged}`, and `RuntimeAdapters::redraw_indicator_surfaces`.

- [ ] **Step 1: Extract a testable redraw reason decision**

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RedrawReason { Metrics, Preferences, Appearance, Fonts, Screens }

#[test]
fn visual_events_redraw_without_sampling_or_saving() {
    for reason in [RedrawReason::Appearance, RedrawReason::Fonts, RedrawReason::Screens] {
        assert_eq!(decision_for(reason), RuntimeDecision { sample: false, save: false, redraw: true });
    }
}
```

- [ ] **Step 2: Implement retained native observers**

```rust
pub struct VisualEnvironment {
    pub appearance: IndicatorAppearance,
    pub increase_contrast: bool,
    pub differentiate_without_color: bool,
    pub reduce_transparency: bool,
}

pub struct VisualEnvironmentObserver {
    notification_tokens: Vec<Retained<NSObject>>,
    button_observer: Option<Retained<NSObject>>,
    proxy: EventLoopProxy<RuntimeEvent>,
}
```

Observe the status button's effective appearance, `NSWorkspaceAccessibilityDisplayOptionsDidChangeNotification`, `NSSystemColorsDidChangeNotification`, `NSFontSetChangedNotification`, and `NSApplicationDidChangeScreenParametersNotification`. Every callback only sends a typed `RuntimeEvent`; it never redraws, samples, or persists directly. Remove/rebind button observation when the status button changes.

- [ ] **Step 3: Extract sampling from drawing in the runtime**

`RuntimeSamplers::poll_due` updates the core and returns effects only. Add:

```rust
impl RuntimeAdapters {
    fn redraw_indicator_surfaces(
        &mut self,
        core: &StatletCore,
        renderer: &mut Renderer,
        button: Option<&NSStatusBarButton>,
    );
}
```

Compose the status scene for the button's effective appearance, then render Light and Dark only when the preferences window has been created. Pass images, fallback state, contrast warnings, and layout diagnostics to the window. Do not sample.

- [ ] **Step 4: Wire every redraw trigger**

Call `redraw_indicator_surfaces` after due metric sampling, `AppEffect::RedrawIndicator`, creation/reopening of Preferences, visual environment change, font-set change, and screen-parameter change. A font-set event refreshes the catalog and invalidates renderer slots before redraw.

- [ ] **Step 5: Verify runtime tests and absence of extra timer constants**

Run: `rtk cargo test --bin statlet && rtk rg -n 'METRICS_REFRESH|preview.*timer|Timer' src/main.rs src/macos`

Expected: tests PASS; `METRICS_REFRESH` and preview timer implementations have no matches.

- [ ] **Step 6: Commit event-driven redraw orchestration**

```bash
rtk git add src/main.rs src/macos/mod.rs src/macos/environment.rs src/macos/windows.rs
rtk git commit -m "feat: redraw indicator for visual changes"
```

### Task 8: Mechanically split native windows and add the preferences shell

**Files:**
- Modify: `src/macos/windows.rs`
- Create: `src/macos/windows/common.rs`
- Create: `src/macos/windows/history.rs`
- Create: `src/macos/windows/free_space.rs`
- Create: `src/macos/windows/preferences/mod.rs`
- Modify: `Cargo.toml`
- Modify: `tests/lifecycle_accessibility.rs`

**Interfaces:**
- Produces: a small `WindowManager`, retained `PreferencesWindow`, `PreferencesArea`, `PreferencesAreaState`, and unchanged `HistoryWindow`/`FreeSpaceWindow` behavior.

- [ ] **Step 1: Record the existing window regression baseline**

Run: `rtk cargo test --test lifecycle_accessibility --test history_flow --test notification_mole_flow`

Expected: PASS before extraction.

- [ ] **Step 2: Mechanically extract common, history, and free-space code**

Move code without changing copy, frames, actions, accessibility labels, reuse, or state application. `windows.rs` retains only `WindowManager`, module declarations, and delegation. Run the baseline command after each extracted window; expected PASS each time.

- [ ] **Step 3: Enable native layout/navigation features**

Add `NSSegmentedControl`, `NSStackView`, `NSScrollView`, `NSLayoutConstraint`, `NSLayoutAnchor`, `NSImageView`, and `NSEvent` features to `objc2-app-kit`. Keep the existing dependency versions.

- [ ] **Step 4: Add a failing pure area-selection test**

```rust
#[test]
fn selecting_an_area_shows_exactly_one_preferences_page() {
    let state = PreferencesAreaState::new();
    assert_eq!(state.visible(), PreferencesArea::Indicator);
    assert_eq!(state.select(PreferencesArea::DiskAndMole).visible(), PreferencesArea::DiskAndMole);
}
```

- [ ] **Step 5: Create the `680 × 700` preferences shell**

```rust
pub struct PreferencesWindow {
    window: Retained<NSWindow>,
    area_selector: Retained<NSSegmentedControl>,
    indicator: IndicatorPage,
    disk_and_mole: DiskAndMolePage,
    footer: PreferencesFooter,
    delegate: Retained<PreferencesWindowDelegate>,
}

impl PreferencesWindow {
    pub fn new(mtm: MainThreadMarker, proxy: EventLoopProxy<RuntimeEvent>) -> Self;
    pub fn apply(&self, state: &AppState, previews: Option<&PreviewImages>);
    pub fn is_created_and_visible(&self) -> bool;
}
```

Use a centered `NSSegmentedControl` with `Indicador` and `Disco e Mole`. Alternate two retained page views with `setHidden`; default to `Indicador`. Put the indicator groups in a scroll view, but keep previews and footer fixed. Rebuild the existing disk/Mole controls in the second page with unchanged events and enablement.

- [ ] **Step 6: Give the shell stable accessibility identifiers**

At minimum: `preferences.area`, `indicator.preview.light`, `indicator.preview.dark`, `indicator.reset.all`, `indicator.reset.undo`, and `indicator.save.retry`. Hide preview images themselves from the accessibility tree; expose separate textual descriptions/warnings.

- [ ] **Step 7: Run window regressions and inspect the diff for accidental behavior changes**

Run: `rtk cargo test --test lifecycle_accessibility --test history_flow --test notification_mole_flow && rtk git diff --check`

Expected: PASS; no disk/history/free-space behavior changes.

- [ ] **Step 8: Commit the native window split and shell**

```bash
rtk git add Cargo.toml Cargo.lock src/macos/windows.rs src/macos/windows tests/lifecycle_accessibility.rs
rtk git commit -m "refactor: prepare preferences window sections"
```

### Task 9: Reusable color editor and CPU/RAM/label groups

**Files:**
- Create: `src/preferences_view.rs`
- Modify: `src/lib.rs`
- Create: `src/macos/windows/preferences/color_editor.rs`
- Create: `src/macos/windows/preferences/indicator.rs`
- Modify: `src/macos/windows/preferences/mod.rs`
- Modify: `Cargo.toml`
- Create: `tests/preferences_view.rs`

**Interfaces:**
- Consumes: Task 4 typed changes and Task 8 shell.
- Produces: `HexDraft`, `ColorEditorState`, `ColorEditor`, CPU/RAM mode controls, label visibility/mode controls, and appearance-variant disclosure.

- [ ] **Step 1: Write failing transient hex-draft tests**

```rust
#[test]
fn incomplete_or_invalid_draft_keeps_the_last_valid_color() {
    let valid = SrgbColor::parse_hex("#34C759").unwrap();
    let mut draft = HexDraft::new(valid);
    assert_eq!(draft.edit("#34C7"), HexEdit::Incomplete);
    assert_eq!(draft.valid_color(), valid);
    assert_eq!(draft.commit(), Err(HexDraftError::ExpectedSixDigits));
    assert_eq!(draft.valid_color(), valid);
}

#[test]
fn six_valid_digits_apply_and_normalize_immediately() {
    let mut draft = HexDraft::new(SrgbColor::parse_hex("#34C759").unwrap());
    assert_eq!(draft.edit("0a84ff"), HexEdit::Applied(SrgbColor::parse_hex("#0A84FF").unwrap()));
    assert_eq!(draft.text(), "#0A84FF");
}
```

- [ ] **Step 2: Implement pure hex/disclosure view state**

`HexDraft` owns transient text, last valid color, and an optional inline error. It emits a domain color only after six valid digits. `ColorEditorState` exposes whether one shared row or Light/Dark rows are visible and retains drafts independently.

- [ ] **Step 3: Enable and build native color controls**

Enable `NSColorWell` and `NSColorSpace`. Implement:

```rust
struct ColorRow {
    well: Retained<NSColorWell>,
    hex: Retained<NSTextField>,
    error: Retained<NSTextField>,
}

pub struct ColorEditor {
    view: Retained<NSStackView>,
    shared: ColorRow,
    light: ColorRow,
    dark: ColorRow,
    variants_toggle: Retained<NSButton>,
    target: Retained<ColorEditorTarget>,
}

pub enum ColorBinding {
    MetricShared(MetricKind),
    MetricAppearance(MetricKind, IndicatorAppearance),
    LabelShared,
    LabelAppearance(IndicatorAppearance),
}

impl ColorEditor {
    pub fn new(mtm: MainThreadMarker, binding: ColorBinding, proxy: EventLoopProxy<RuntimeEvent>) -> Self;
    pub fn apply(&self, state: &ColorEditorState);
}
```

Use minimal `NSColorWell`, `setSupportsAlpha(false)`, explicit conversion through `NSColorSpace::sRGBColorSpace()`, uppercase field normalization, Return/blur validation, and inline accessible error text. Retain every target/delegate because AppKit target references are weak. Programmatic `apply` must suppress actions.

When the preferences window closes or switches away from Indicator, call `deactivate()` on every active well before hiding the page so the shared `NSColorPanel` cannot keep a stale target.

- [ ] **Step 4: Build CPU and RAM color sections**

Each metric uses a `Dinâmica | Fixa` segmented control. Fixed reveals its `ColorEditor`. `Personalizar claro e escuro` reveals both appearance rows without deleting their values when disabled. Map actions to the exact `IndicatorPreferenceChange` variants from Task 4.

- [ ] **Step 5: Build the labels section**

Add one `Mostrar rótulos C/R` switch and `Neutra | Igual ao valor | Personalizada`. Only Custom reveals its `ColorEditor`. Hiding labels does not disable the stored label-color preference.

- [ ] **Step 6: Add stable accessibility identifiers and keyboard order**

Use `indicator.cpu.mode`, `indicator.cpu.color.hex`, `indicator.ram.mode`, `indicator.ram.color.hex`, `indicator.labels.visible`, `indicator.labels.mode`, and `indicator.labels.color.hex`. Order Tab as mode → well → hex → next metric/group. Space/Return opens the well; Return validates hex.

- [ ] **Step 7: Run pure UI/reducer tests and native compile tests**

Run: `rtk cargo test --test preferences_view --test indicator_preferences_flow --test indicator_presentation && rtk cargo test --bin statlet`

Expected: PASS.

- [ ] **Step 8: Commit color and label controls**

```bash
rtk git add Cargo.toml Cargo.lock src/lib.rs src/preferences_view.rs src/macos/windows/preferences tests/preferences_view.rs
rtk git commit -m "feat: customize indicator colors and labels"
```

### Task 10: Font picker, typography controls, and interval editor

**Files:**
- Create: `src/macos/windows/preferences/font_picker.rs`
- Modify: `src/macos/windows/preferences/indicator.rs`
- Modify: `src/preferences_view.rs`
- Modify: `Cargo.toml`
- Modify: `tests/preferences_view.rs`

**Interfaces:**
- Consumes: `FontCatalog`, typography preferences, refresh interval, typed reducer changes.
- Produces: searchable `FontPicker`, `IntervalDraft`, typography group, and update group.

- [ ] **Step 1: Write failing font-filter and interval-draft tests**

```rust
#[test]
fn font_filter_is_case_insensitive_sorted_and_keeps_missing_selection_visible() {
    let result = filter_font_families(
        &["Menlo".into(), "Avenir Next".into()],
        "ave",
        Some("Missing Family"),
    );
    assert_eq!(result, vec![FontRow::Missing("Missing Family".into()), FontRow::Available("Avenir Next".into())]);
}

#[test]
fn interval_draft_applies_only_whole_values_from_one_through_sixty() {
    let mut draft = IntervalDraft::new(MetricsRefreshInterval::default());
    assert!(draft.commit("0").is_err());
    assert!(draft.commit("1.5").is_err());
    assert_eq!(draft.commit("60").unwrap().seconds(), 60);
    assert!(draft.commit("61").is_err());
}
```

- [ ] **Step 2: Implement pure font filtering and interval draft state**

Sort with case-insensitive comparison, keep `System Monospaced` first, show a requested missing family as a distinct fallback row, and emit no preference change from invalid interval text.

- [ ] **Step 3: Enable table/search/stepper native features**

Add `NSSearchField`, `NSTableView`, `NSTableColumn`, `NSStepper`, and `block2` to `objc2-app-kit` features. Do not introduce a third-party font picker.

- [ ] **Step 4: Implement the searchable font sheet**

```rust
pub struct FontPicker {
    sheet: Retained<NSWindow>,
    search: Retained<NSSearchField>,
    table: Retained<NSTableView>,
    data_source: Retained<FontPickerDataSource>,
    delegate: Retained<FontPickerDelegate>,
    proxy: EventLoopProxy<RuntimeEvent>,
}

impl FontPicker {
    pub fn present(
        &mut self,
        parent: &NSWindow,
        catalog: &FontCatalog,
        selected: &FontFamilyPreference,
    );
    pub fn refresh_catalog(&mut self, catalog: &FontCatalog);
}
```

Use a headerless, single-selection `NSTableView`; each row shows family name and `C 42% / R 68%` in that family. Arrow keys navigate. Selection emits `SetFontFamily` immediately and closes the sheet. A missing saved family remains named on the launch button with a fallback warning.

- [ ] **Step 5: Add size and weight controls**

Use an integer control for `9..=14 pt` and a `Regular | Médio | Negrito` segmented control. Map only validated values to `SetFontSize`/`SetFontWeight`. Show layout warnings supplied by the renderer and `Restaurar tipografia`; never auto-correct a wide/tall choice.

- [ ] **Step 6: Add synchronized interval field and stepper**

Use a numeric `NSTextField`, `NSStepper(min=1,max=60,increment=1,wraps=false)`, visible `segundos`, and help text explaining lower interval/higher resource use. A valid edit emits `SetRefreshInterval`; invalid Return/blur shows inline range text and preserves the last valid schedule.

- [ ] **Step 7: Add accessibility identifiers**

Use `indicator.font.family`, `indicator.font.search`, `indicator.font.size`, `indicator.font.weight`, and `indicator.refresh.interval`. The font sample is descriptive, not the only accessible name.

- [ ] **Step 8: Run UI model and runtime schedule tests**

Run: `rtk cargo test --test preferences_view --test indicator_preferences_flow --test metrics_sampling_schedule && rtk cargo test --bin statlet`

Expected: PASS.

- [ ] **Step 9: Commit typography and interval controls**

```bash
rtk git add Cargo.toml Cargo.lock src/preferences_view.rs src/macos/windows/preferences tests/preferences_view.rs
rtk git commit -m "feat: customize indicator font and refresh"
```

### Task 11: Preview warnings, reset/undo, save recovery, and accessibility completion

**Files:**
- Create: `src/macos/windows/preferences/preview.rs`
- Modify: `src/macos/windows/preferences/mod.rs`
- Modify: `src/macos/windows/preferences/indicator.rs`
- Modify: `src/macos/windows/common.rs`
- Modify: `src/main.rs`
- Modify: `tests/indicator_preferences_flow.rs`
- Modify: `tests/lifecycle_accessibility.rs`

**Interfaces:**
- Consumes: `PreviewImages`, `LayoutDiagnostics`, font fallback, reducer undo/save state.
- Produces: visible light/dark previews, contrast warnings, group/global resets, Command-Z undo, retry save, and complete accessibility state.

- [ ] **Step 1: Add pure contrast tests for representative preview backgrounds**

```rust
#[test]
fn small_text_below_four_point_five_to_one_warns_without_replacing_color() {
    let chosen = SrgbColor::parse_hex("#777777").unwrap();
    let warning = contrast_warning(chosen, PreviewBackground::Light);
    assert!(warning.is_some());
    assert_eq!(chosen.to_hex(), "#777777");
}
```

Use WCAG relative luminance in sRGB, representative backgrounds `#FFFFFF` and `#1E1E1E`, and threshold `4.5`. The warning is advisory and never writes preferences.

- [ ] **Step 2: Implement the preview pane**

```rust
pub struct PreviewPane {
    view: Retained<NSStackView>,
    light_image: Retained<NSImageView>,
    dark_image: Retained<NSImageView>,
    summary: Retained<NSTextField>,
    warnings: Retained<NSTextField>,
}

impl PreviewPane {
    pub fn apply(
        &self,
        images: &PreviewImages,
        layout: &LayoutDiagnostics,
        fallback: Option<&FontResolution>,
        environment: &VisualEnvironment,
    );
}
```

Present Light/Dark side by side at menu-bar scale. Hide raw `NSImageView`s from accessibility and expose textual preview summaries, contrast limitations, layout warnings, and fallback state. Mention wallpaper/transparency/state limitations.

- [ ] **Step 3: Wire group resets and confirmed global reset**

Each group button sends `ResetIndicatorGroup` without confirmation. `Restaurar indicador aos padrões…` uses `NSAlert` and sends `ResetIndicatorConfirmed` only from its destructive first button. It never changes the disk/Mole page.

- [ ] **Step 4: Wire one-level undo and window-close discard**

Show `Desfazer restauração` only while `can_undo_indicator_reset`. Use a retained `PreferencesWindowHost` subclass whose `performKeyEquivalent` intercepts Command-Z only while that flag is true and sends the same `UndoIndicatorReset` as the visible button; otherwise it delegates to `super`, preserving ordinary text-field undo. A window delegate sends `PreferencesWindowClosed` when the preferences window closes.

- [ ] **Step 5: Wire save error, explicit retry, and success clearing**

When `PreferencesSaveStatus::Failed`, show `Não foi possível salvar as preferências.` and `Tentar novamente`. Retry sends `RetrySavePreferences`, which saves the current complete document. A successful adapter save sends `PreferencesSaveFinished(Saved)` and hides the message. Keep the concrete `io::Error` only in stderr.

- [ ] **Step 6: Complete environment-driven accessibility responses**

On Increase Contrast, re-resolve semantic colors; on Differentiate Without Color, retain values and symbolic badges; on Reduce Transparency, refresh preview background/warning text. Neither sampling nor persistence occurs. Ensure the status accessibility label remains complete with C/R hidden and fixed colors.

- [ ] **Step 7: Extend lifecycle/accessibility contract tests**

Add tests for hidden labels retaining the full accessible label, launch/reopen retaining one preferences window, save failure not closing it, and closing clearing undo without changing preferences.

- [ ] **Step 8: Run automated tests and perform focused native checks**

Run: `rtk cargo test --test indicator_preferences_flow --test lifecycle_accessibility --test indicator_presentation && rtk cargo test --bin statlet`

Then manually verify: Tab/Shift-Tab order; Space/Return on color well; invalid/pasted hex; no alpha; font arrow navigation; stepper boundaries; each reset; confirmed global reset; button and Command-Z undo with focus inside/outside a text field; simulated unwritable preferences path and retry.

- [ ] **Step 9: Commit recovery, previews, and accessibility**

```bash
rtk git add src/main.rs src/macos/windows tests/indicator_preferences_flow.rs tests/lifecycle_accessibility.rs
rtk git commit -m "feat: complete indicator preferences experience"
```

### Task 12: Product documentation, automated gates, native validation, and performance evidence

**Files:**
- Modify: `README.md`
- Modify: `docs/product/v1.md`
- Modify: `docs/validation/accessibility-lifecycle.md`
- Create: `docs/validation/indicator-customization.md`
- Modify: `scripts/measure-soak.sh` only if the v2 nested interval needs explicit baseline verification.
- Modify: `tests/package_contract.sh` only if a v2 fixture is required in addition to the existing v1 migration fixture.

**Interfaces:**
- Consumes: all completed behavior.
- Produces: truthful public behavior, reproducible validation checklist, package proof, and default-interval soak evidence.

- [ ] **Step 1: Update product documentation without rewriting v1 history**

Document the next-version customization separately from the immutable v1.0.0 release notes. In `docs/product/v1.md`, annotate the former out-of-scope interval/preset statement as v1-specific and link to the approved customization spec. Update README claims only for behavior present in the built branch.

- [ ] **Step 2: Add a concrete validation document**

`docs/validation/indicator-customization.md` must list automated evidence and unchecked manual gates for color/hex, Light/Dark, contrast limitations, any font, missing/reinstalled font, 9–14 pt, weights, 1/2/60 s, resets/undo, save retry, VoiceOver, Full Keyboard Access, Increase Contrast, Differentiate Without Color, Reduce Transparency, notch, Retina, display change, sleep/wake, and soak.

- [ ] **Step 3: Run formatting, shell syntax, tests, and strict Clippy**

```bash
rtk cargo fmt --all -- --check
rtk bash -n scripts/*.sh tests/package_contract.sh
rtk cargo test --all-targets --all-features --locked
rtk cargo clippy --all-targets --all-features --locked -- -D warnings
rtk git diff --check
```

Expected: all PASS with no warnings.

- [ ] **Step 4: Build and verify the exact production bundle**

Run: `rtk bash tests/package_contract.sh`

Expected: bundle, architecture, Info.plist, privacy manifest, licenses, signing mode, ZIP extraction, and checksum contract all PASS. Preserve the v1 preferences fixture to prove migration; add a v2 fixture only when it verifies a new package boundary rather than duplicating unit tests.

- [ ] **Step 5: Perform native visual and accessibility validation**

Run the built app with isolated test preferences, then complete the checklist from Step 2. Capture exact macOS version, hardware/display setup, tested fonts and sizes, VoiceOver/Full Keyboard Access status, and any unexecuted gates. Do not claim unavailable hardware/assistive validation.

- [ ] **Step 6: Run a 30-minute default configuration soak**

Ensure Mole is disabled and v2 `refreshInterval` is exactly 2. Then run:

```bash
rtk bash scripts/measure-soak.sh dist/Statlet.app 1800 10 dist/soak-indicator-default
```

Compare mean CPU, RSS growth, peak RSS, physical footprint, idle wakeups, and context switches against the v1 evidence. Investigate any failure of existing gates or material regression before completion; do not normalize away real wakeups.

- [ ] **Step 7: Review the complete diff against the approved spec**

Run: `rtk git diff 14ecc8746d7b326cd03f70534395b96bacd7def5 --stat && rtk git diff 14ecc8746d7b326cd03f70534395b96bacd7def5 -- docs/superpowers/specs/2026-08-12-indicator-customization-design.md`

Expected: implementation covers the spec and does not alter the approved design document. Confirm no `.superpowers/` path is tracked.

- [ ] **Step 8: Commit documentation and validation evidence**

```bash
rtk git add README.md docs/product/v1.md docs/validation scripts/measure-soak.sh tests/package_contract.sh
rtk git commit -m "docs: validate indicator customization"
```

Only add the script/test paths if they actually changed. Do not commit unavailable manual checks as passed.

## Final Review Checklist

- [ ] Every task has its own red/green test cycle and reviewer-sized commit.
- [ ] `Preferences`/`AppEffect` non-`Copy` changes do not leave implicit moves or stale `.copied()` calls.
- [ ] v1 migration preserves Mole and threshold and does not save until the next real preference write.
- [ ] Invalid drafts never enter the reducer or JSON.
- [ ] Dynamic defaults visually match v1 and use the same metric/severity calculations.
- [ ] Metric interval changes never delay the independent disk schedule.
- [ ] Real status and both previews use the same composer, font resolver, layout, and renderer.
- [ ] No preview timer, new polling loop, or permanent worker exists.
- [ ] Missing fonts fall back without overwriting the requested family and recover after font notification.
- [ ] C/R can only be hidden together, and hiding them preserves the complete accessible label.
- [ ] Reset-all never modifies Disk/Mole, and undo is one-level transient state.
- [ ] Save failure remains visible, retry saves the latest full document, and success clears the warning.
- [ ] Automated gates, package contract, manual residual gates, and soak evidence are reported truthfully.
