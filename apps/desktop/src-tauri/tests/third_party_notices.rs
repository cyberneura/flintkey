//! THIRD-PARTY-NOTICES.txt must list every direct dependency that ships in the app. Regenerate it with
//! `scripts/generate-third-party-notices.sh` when this fails.

use std::path::Path;

const ROOT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/../../..");

fn read(path: &str) -> String {
    std::fs::read_to_string(Path::new(ROOT).join(path)).unwrap_or_else(|error| panic!("{path}: {error}"))
}

/// The package names in the `[dependencies]` tables (including target-specific ones) of a Cargo.toml,
/// leaving out path dependencies, which are Flintkey's own crates.
fn cargo_dependencies(manifest: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_dependencies = false;
    for line in manifest.lines().map(str::trim) {
        if line.starts_with('[') {
            in_dependencies = line == "[dependencies]" || line.ends_with(".dependencies]");
            continue;
        }
        if !in_dependencies || line.is_empty() || line.starts_with('#') || line.contains("path =") {
            continue;
        }
        if let Some((name, _)) = line.split_once('=') {
            names.push(name.trim().to_string());
        }
    }
    names
}

fn npm_dependencies(package_json: &str) -> Vec<String> {
    let json: serde_json::Value = serde_json::from_str(package_json).unwrap();
    json["dependencies"]
        .as_object()
        .map_or_else(Vec::new, |deps| deps.keys().cloned().collect())
}

/// The names in the "Used by:" blocks.
fn listed(notices: &str) -> Vec<String> {
    let mut names = Vec::new();
    let mut in_used_by = false;
    for line in notices.lines() {
        if line == "Used by:" {
            in_used_by = true;
        } else if in_used_by && line.starts_with("  ") {
            names.extend(line.split_whitespace().next().map(str::to_string));
        } else {
            in_used_by = false;
        }
    }
    names
}

#[test]
fn every_direct_dependency_is_in_the_notices() {
    // Arrange
    let notices = listed(&read("THIRD-PARTY-NOTICES.txt"));
    let mut dependencies = cargo_dependencies(&read("crates/flintkey-core/Cargo.toml"));
    dependencies.extend(cargo_dependencies(&read("apps/desktop/src-tauri/Cargo.toml")));
    dependencies.extend(npm_dependencies(&read("apps/desktop/package.json")));

    // Act
    let missing: Vec<&String> = dependencies.iter().filter(|name| !notices.contains(name)).collect();

    // Assert
    assert!(!dependencies.is_empty());
    assert!(missing.is_empty(), "not in THIRD-PARTY-NOTICES.txt: {missing:?}");
}

#[test]
fn dependency_tables_are_read_and_path_dependencies_left_out() {
    // Arrange
    let manifest = "[package]\nname = \"x\"\n\n[dependencies]\nown = { path = \"../own\" }\nserde = \"1\"\n# note\n\n[target.'cfg(windows)'.dependencies]\nwindows = \"0.61\"\n\n[dev-dependencies]\ntempfile = \"3\"\n";

    // Act
    let names = cargo_dependencies(manifest);

    // Assert
    assert_eq!(names, ["serde", "windows"]);
}
