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
}

#[test]
fn standard_features_keep_sidecar_opt_in() {
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
}

#[test]
fn desktop_feature_is_the_explicit_sidecar_group() {
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
