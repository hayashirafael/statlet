use objc2::rc::Retained;
use objc2::MainThreadMarker;
use objc2_app_kit::{NSFont, NSFontManager, NSFontTraitMask};

use statlet::indicator_preferences::{FontFamilyPreference, FontWeight, TypographyPreferences};

pub struct FontResolution {
    pub font: Retained<NSFont>,
    pub requested_family: FontFamilyPreference,
    pub resolved_family: String,
    pub used_fallback: bool,
}

pub struct FontCatalog {
    manager: Retained<NSFontManager>,
    families: FamilyCache<NativeFamilySource>,
}

impl FontCatalog {
    pub fn new(marker: MainThreadMarker) -> Self {
        let manager = NSFontManager::sharedFontManager(marker);
        Self {
            families: FamilyCache::new(NativeFamilySource {
                manager: manager.clone(),
            }),
            manager,
        }
    }

    pub fn families(&self) -> &[String] {
        self.families.families()
    }

    pub fn resolve(&self, preferences: &TypographyPreferences) -> FontResolution {
        let plan = self.families.resolution_plan(preferences);
        let size = f64::from(preferences.size.points());
        let named_font = match &plan.resolved_family {
            ResolvedFamily::SystemMonospaced => None,
            ResolvedFamily::Named(family) => self.manager.fontWithFamily_traits_weight_size(
                &objc2_foundation::NSString::from_str(family),
                NSFontTraitMask::empty(),
                manager_weight(preferences.weight),
                size,
            ),
        };
        let named_resolution_failed =
            matches!(plan.resolved_family, ResolvedFamily::Named(_)) && named_font.is_none();
        let font = named_font.unwrap_or_else(|| {
            NSFont::monospacedSystemFontOfSize_weight(size, system_weight(preferences.weight))
        });
        let resolved_family = font
            .familyName()
            .map(|family| family.to_string())
            .unwrap_or_default();
        FontResolution {
            font,
            requested_family: plan.requested_family,
            resolved_family,
            used_fallback: plan.used_fallback || named_resolution_failed,
        }
    }

    pub fn refresh(&mut self) {
        self.families.refresh();
    }
}

trait FamilySource {
    fn installed_families(&self) -> Vec<String>;
}

struct NativeFamilySource {
    manager: Retained<NSFontManager>,
}

impl FamilySource for NativeFamilySource {
    fn installed_families(&self) -> Vec<String> {
        self.manager
            .availableFontFamilies()
            .iter()
            .map(|family| family.to_string())
            .collect()
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
enum ResolvedFamily {
    SystemMonospaced,
    Named(String),
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct ResolutionPlan {
    requested_family: FontFamilyPreference,
    resolved_family: ResolvedFamily,
    used_fallback: bool,
}

struct FamilyCache<S> {
    source: S,
    families: Vec<String>,
}

impl<S: FamilySource> FamilyCache<S> {
    fn new(source: S) -> Self {
        let families = normalized_families(source.installed_families());
        Self { source, families }
    }

    fn families(&self) -> &[String] {
        &self.families
    }

    fn resolution_plan(&self, preferences: &TypographyPreferences) -> ResolutionPlan {
        let (resolved_family, used_fallback) = match &preferences.family {
            FontFamilyPreference::SystemMonospaced => (ResolvedFamily::SystemMonospaced, false),
            FontFamilyPreference::Named(requested) => self
                .canonical_family(requested)
                .map(|family| (ResolvedFamily::Named(family.to_owned()), false))
                .unwrap_or((ResolvedFamily::SystemMonospaced, true)),
        };
        ResolutionPlan {
            requested_family: preferences.family.clone(),
            resolved_family,
            used_fallback,
        }
    }

    fn refresh(&mut self) {
        self.families = normalized_families(self.source.installed_families());
    }

    fn canonical_family(&self, requested: &str) -> Option<&str> {
        let requested = folded_family(requested);
        self.families
            .iter()
            .find(|family| folded_family(family) == requested)
            .map(String::as_str)
    }
}

fn normalized_families(families: Vec<String>) -> Vec<String> {
    let mut families = families
        .into_iter()
        .map(|family| family.trim().to_owned())
        .filter(|family| !family.is_empty() && !family.starts_with('.'))
        .collect::<Vec<_>>();
    families.sort_by(|left, right| {
        folded_family(left)
            .cmp(&folded_family(right))
            .then_with(|| left.cmp(right))
    });
    families.dedup_by(|left, right| folded_family(left) == folded_family(right));
    families
}

fn folded_family(family: &str) -> String {
    family.to_lowercase()
}

fn manager_weight(weight: FontWeight) -> isize {
    match weight {
        FontWeight::Regular => 5,
        FontWeight::Medium => 6,
        FontWeight::Bold => 9,
    }
}

fn system_weight(weight: FontWeight) -> f64 {
    unsafe {
        match weight {
            FontWeight::Regular => objc2_app_kit::NSFontWeightRegular,
            FontWeight::Medium => objc2_app_kit::NSFontWeightMedium,
            FontWeight::Bold => objc2_app_kit::NSFontWeightBold,
        }
    }
}

#[cfg(test)]
mod tests {
    use std::cell::RefCell;
    use std::rc::Rc;

    use statlet::indicator_preferences::{FontFamilyPreference, FontWeight, TypographyPreferences};

    use super::*;

    #[derive(Clone)]
    struct FakeFamilySource {
        families: Rc<RefCell<Vec<String>>>,
    }

    impl FakeFamilySource {
        fn new(families: &[&str]) -> Self {
            Self {
                families: Rc::new(RefCell::new(
                    families.iter().map(|family| (*family).to_owned()).collect(),
                )),
            }
        }

        fn replace(&self, families: &[&str]) {
            *self.families.borrow_mut() =
                families.iter().map(|family| (*family).to_owned()).collect();
        }
    }

    impl FamilySource for FakeFamilySource {
        fn installed_families(&self) -> Vec<String> {
            self.families.borrow().clone()
        }
    }

    fn preferences(family: FontFamilyPreference) -> TypographyPreferences {
        let mut preferences =
            statlet::indicator_preferences::IndicatorPreferences::default().typography;
        preferences.family = family;
        preferences
    }

    #[test]
    fn missing_named_family_uses_system_fallback_without_rewriting_request() {
        let cache = FamilyCache::new(FakeFamilySource::new(&["Menlo"]));
        let requested = FontFamilyPreference::named("Statlet Definitely Missing").unwrap();

        let plan = cache.resolution_plan(&preferences(requested.clone()));

        assert_eq!(plan.requested_family, requested);
        assert_eq!(plan.resolved_family, ResolvedFamily::SystemMonospaced);
        assert!(plan.used_fallback);
    }

    #[test]
    fn system_default_and_installed_family_resolve_without_fallback() {
        let cache = FamilyCache::new(FakeFamilySource::new(&["Avenir Next", "Menlo"]));

        let system = cache.resolution_plan(&preferences(FontFamilyPreference::SystemMonospaced));
        let named = cache.resolution_plan(&preferences(
            FontFamilyPreference::named("Avenir Next").unwrap(),
        ));

        assert_eq!(system.resolved_family, ResolvedFamily::SystemMonospaced);
        assert!(!system.used_fallback);
        assert_eq!(
            named.resolved_family,
            ResolvedFamily::Named("Avenir Next".to_owned())
        );
        assert!(!named.used_fallback);
    }

    #[test]
    fn installed_family_matches_case_insensitively_using_catalog_spelling() {
        let cache = FamilyCache::new(FakeFamilySource::new(&["Menlo"]));
        let requested = FontFamilyPreference::named("menlo").unwrap();

        let plan = cache.resolution_plan(&preferences(requested.clone()));

        assert_eq!(plan.requested_family, requested);
        assert_eq!(
            plan.resolved_family,
            ResolvedFamily::Named("Menlo".to_owned())
        );
        assert!(!plan.used_fallback);
    }

    #[test]
    fn semantic_weights_map_to_nearest_appkit_weight_requests() {
        assert_eq!(manager_weight(FontWeight::Regular), 5);
        assert_eq!(manager_weight(FontWeight::Medium), 6);
        assert_eq!(manager_weight(FontWeight::Bold), 9);
    }

    #[test]
    fn catalog_filters_hidden_and_blank_families_then_orders_case_insensitively() {
        let cache = FamilyCache::new(FakeFamilySource::new(&[
            "zeta",
            ".Apple Hidden",
            "",
            "Beta",
            "alpha",
            "  ",
        ]));

        assert_eq!(cache.families(), ["alpha", "Beta", "zeta"]);
    }

    #[test]
    fn refresh_invalidates_cached_families_and_recovers_a_reinstalled_request() {
        let source = FakeFamilySource::new(&["Menlo"]);
        let mut cache = FamilyCache::new(source.clone());
        let requested = FontFamilyPreference::named("Iosevka").unwrap();
        let typography = preferences(requested.clone());

        assert!(cache.resolution_plan(&typography).used_fallback);
        source.replace(&["Menlo", "Iosevka"]);
        assert!(
            cache.resolution_plan(&typography).used_fallback,
            "discovery remains cached until an explicit refresh"
        );

        cache.refresh();
        let recovered = cache.resolution_plan(&typography);
        assert_eq!(recovered.requested_family, requested);
        assert_eq!(
            recovered.resolved_family,
            ResolvedFamily::Named("Iosevka".to_owned())
        );
        assert!(!recovered.used_fallback);
    }

    #[test]
    fn native_resolution_runs_only_when_the_test_has_a_main_thread_marker() {
        let Some(marker) = objc2::MainThreadMarker::new() else {
            eprintln!("SKIP: AppKit font resolution requires a main-thread test marker");
            return;
        };
        let catalog = FontCatalog::new(marker);
        let requested = statlet::indicator_preferences::IndicatorPreferences::default().typography;

        let resolved = catalog.resolve(&requested);

        assert_eq!(resolved.requested_family, requested.family);
        assert!(!resolved.used_fallback);
    }
}
