use toml::Value;

fn manifest() -> Value {
    toml::from_str(include_str!("../Cargo.toml")).expect("Cargo.toml should parse as TOML")
}

fn feature_members(manifest: &Value, feature: &str) -> Vec<String> {
    manifest["features"][feature]
        .as_array()
        .unwrap_or_else(|| panic!("feature '{feature}' should exist"))
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| panic!("feature '{feature}' should contain strings"))
                .to_string()
        })
        .collect()
}

#[test]
fn default_and_minimal_features_are_telegram_only() {
    let manifest = manifest();

    assert_eq!(
        feature_members(&manifest, "default"),
        vec!["channel-telegram"]
    );
    assert_eq!(
        feature_members(&manifest, "minimal"),
        vec!["channel-telegram"]
    );

    let default = feature_members(&manifest, "default");
    assert!(!default.contains(&"channel-discord".to_string()));
    assert!(!default.contains(&"tool-discord".to_string()));
    assert!(!default.contains(&"computer-use-sidecar".to_string()));
    assert!(!default.contains(&"desktop".to_string()));
}

#[test]
fn standard_features_expose_discord_without_desktop() {
    let manifest = manifest();
    let standard = feature_members(&manifest, "standard");

    assert_eq!(
        standard,
        vec![
            "channel-telegram",
            "channel-discord",
            "web-fetch-html2md",
            "tool-discord",
        ]
    );
    assert!(!standard.contains(&"computer-use-sidecar".to_string()));
    assert!(!standard.contains(&"desktop".to_string()));
}

#[test]
fn desktop_and_full_features_are_the_explicit_computer_use_path() {
    let manifest = manifest();

    assert_eq!(
        feature_members(&manifest, "desktop"),
        vec!["computer-use-sidecar"]
    );
    assert_eq!(
        feature_members(&manifest, "full"),
        vec!["standard", "desktop"]
    );
}

#[test]
#[cfg(not(feature = "computer-use-sidecar"))]
fn build_without_computer_use_sidecar_does_not_expose_computer_use_feature() {
    let default = feature_members(&manifest(), "default");
    assert!(!default.contains(&"computer-use-sidecar".to_string()));
    assert!(!default.contains(&"desktop".to_string()));
}

#[test]
#[cfg(feature = "computer-use-sidecar")]
fn desktop_or_full_build_exposes_computer_use_feature() {
    assert_eq!(
        feature_members(&manifest(), "desktop"),
        vec!["computer-use-sidecar"]
    );
}

#[test]
#[cfg(not(feature = "tool-discord"))]
fn build_without_tool_discord_does_not_expose_discord_only_tool_feature() {
    let default = feature_members(&manifest(), "default");
    assert!(!default.contains(&"channel-discord".to_string()));
    assert!(!default.contains(&"tool-discord".to_string()));
}

#[test]
#[cfg(feature = "tool-discord")]
fn standard_or_full_build_exposes_discord_tool_feature() {
    let standard = feature_members(&manifest(), "standard");
    assert!(standard.contains(&"channel-discord".to_string()));
    assert!(standard.contains(&"tool-discord".to_string()));
}

#[test]
fn default_provider_visibility_is_core_only_until_advanced_mode() {
    use topclaw::providers::{self, ProviderVisibilityMode};

    assert_eq!(
        providers::visible_provider_ids(ProviderVisibilityMode::Core),
        vec!["openai-codex", "openrouter", "ollama"]
    );

    let advanced = providers::advanced_provider_ids();
    assert!(advanced.contains(&"anthropic"));
    assert!(advanced.contains(&"gemini"));
    for provider_id in providers::visible_provider_ids(ProviderVisibilityMode::Core) {
        assert!(
            providers::is_core_provider(provider_id),
            "{provider_id} should be in the core provider set"
        );
    }
}
