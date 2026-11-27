//! Icons for commands and tags: what an `icon:` value refers to, and the named icons and tag icons in `icons.yaml`.
//!
//! An `icon:` value is one of
//! - a URL (`https://…`),
//! - a path: absolute, or starting with `~/`,
//! - the name of an icon defined in `icons.yaml`,
//! - otherwise, a path inside the `icons` folder of the configuration folder. It may not leave that folder.

use crate::diagnostic::Diagnostic;
use crate::paths;
use serde::Deserialize;
use serde_saphyr::Spanned;
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};

pub const ICONS_FILE: &str = "icons.yaml";
/// The folder, inside the configuration folder, that relative icon paths are read from.
pub const ICONS_DIR: &str = "icons";

/// The contents of `icons.yaml`.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct IconConfig {
    /// Icons that commands and tags can refer to by name. In written order.
    pub named: Vec<NamedIcon>,
    /// Tag → icon value. A command with no icon of its own (and no favicon or app icon) shows the icon of its
    /// first tag that has one.
    pub tags: BTreeMap<String, String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NamedIcon {
    pub name: String,
    /// The `path` or `url` as written.
    pub source: String,
}

/// Where an icon's image is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum IconSource {
    File(PathBuf),
    Url(String),
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawIcons {
    icons: Option<Vec<RawNamed>>,
    tags: Option<BTreeMap<String, Spanned<String>>>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawNamed {
    name: Spanned<String>,
    path: Option<Spanned<String>>,
    url: Option<Spanned<String>>,
}

fn problem_at<T>(spanned: &Spanned<T>, message: String) -> Diagnostic {
    Diagnostic::at(spanned.referenced.line(), spanned.referenced.column(), message)
}

fn is_url(value: &str) -> bool {
    let lowered = value.trim_start().to_ascii_lowercase();
    lowered.starts_with("https://") || lowered.starts_with("http://")
}

/// Reads `icons.yaml`. An empty file is valid.
pub fn parse_icons(source: &str) -> Result<IconConfig, Vec<Diagnostic>> {
    let raw = match serde_saphyr::from_str::<Option<RawIcons>>(source) {
        Ok(Some(raw)) => raw,
        Ok(None) => return Ok(IconConfig::default()),
        Err(error) => return Err(vec![Diagnostic::from_yaml_error(&error)]),
    };
    let mut config = IconConfig::default();
    let mut problems = Vec::new();
    for entry in raw.icons.unwrap_or_default() {
        let name = entry.name.value.trim().to_string();
        if name.is_empty() {
            problems.push(problem_at(&entry.name, "`name` must not be empty".into()));
            continue;
        }
        if config.named.iter().any(|named| named.name == name) {
            problems.push(problem_at(&entry.name, format!("icon `{name}` is defined twice")));
            continue;
        }
        let source = match (entry.path, entry.url) {
            (Some(path), None) if is_url(&path.value) => {
                problems.push(problem_at(
                    &path,
                    format!("icon `{name}`: a URL goes in `url`, not `path`"),
                ));
                continue;
            }
            (Some(path), None) if !path.value.trim().is_empty() => path.value,
            (None, Some(url)) if is_url(&url.value) => url.value,
            (None, Some(url)) => {
                problems.push(problem_at(
                    &url,
                    format!("icon `{name}`: `url` must start with https:// or http://"),
                ));
                continue;
            }
            _ => {
                problems.push(problem_at(
                    &entry.name,
                    format!("icon `{name}` needs one of `path` or `url`"),
                ));
                continue;
            }
        };
        config.named.push(NamedIcon { name, source });
    }
    for (tag, icon) in raw.tags.unwrap_or_default() {
        if icon.value.trim().is_empty() {
            problems.push(problem_at(&icon, format!("the icon of tag `{tag}` must not be empty")));
        } else {
            config.tags.insert(tag, icon.value);
        }
    }
    if problems.is_empty() { Ok(config) } else { Err(problems) }
}

/// What the `icon:` value `value` refers to. `config_dir` is the configuration folder.
pub fn resolve(value: &str, config: &IconConfig, config_dir: &Path, home: &Path) -> Result<IconSource, String> {
    let value = value.trim();
    match config.named.iter().find(|named| named.name == value) {
        // A named icon's own value is never another name, so names cannot loop.
        Some(named) => written(&named.source, config_dir, home),
        None => written(value, config_dir, home),
    }
}

fn written(value: &str, config_dir: &Path, home: &Path) -> Result<IconSource, String> {
    let value = value.trim();
    if is_url(value) {
        return Ok(IconSource::Url(value.to_string()));
    }
    if value == "~" || value.starts_with("~/") || Path::new(value).is_absolute() {
        return Ok(IconSource::File(paths::expand_home(value, home)));
    }
    let relative = Path::new(value);
    if !relative
        .components()
        .all(|component| matches!(component, Component::Normal(_)))
    {
        return Err(format!(
            "icon `{value}`: a path in the {ICONS_DIR} folder may not contain `..` or `.`; write a full path for a file elsewhere"
        ));
    }
    Ok(IconSource::File(config_dir.join(ICONS_DIR).join(relative)))
}

/// Whether `path`, once symlinks are resolved, is still inside the `icons` folder of `config_dir`. A path that was
/// written relative must pass this before it is read, so a link inside the folder cannot reach elsewhere.
pub fn stays_in_icons_dir(path: &Path, config_dir: &Path) -> bool {
    let (Ok(path), Ok(dir)) = (path.canonicalize(), config_dir.join(ICONS_DIR).canonicalize()) else {
        return false;
    };
    path.starts_with(dir)
}

/// Whether an `icon:` value was written as a path relative to the `icons` folder (and so must stay inside it).
pub fn is_in_icons_dir(value: &str, config: &IconConfig) -> bool {
    let value = value.trim();
    let written = config
        .named
        .iter()
        .find(|named| named.name == value)
        .map_or(value, |named| named.source.as_str());
    !is_url(written) && !written.starts_with('~') && !Path::new(written).is_absolute()
}

/// The icon of the first of `tags` that has one.
pub fn tag_icon<'a>(tags: &[String], config: &'a IconConfig) -> Option<&'a str> {
    tags.iter().find_map(|tag| config.tags.get(tag)).map(String::as_str)
}

#[cfg(test)]
mod tests {
    use super::*;

    const CONFIG: &str = "/home/me/.config/flintkey";

    fn resolved(value: &str, config: &IconConfig) -> Result<IconSource, String> {
        resolve(value, config, Path::new(CONFIG), Path::new("/home/me"))
    }

    fn file(path: &str) -> Result<IconSource, String> {
        Ok(IconSource::File(PathBuf::from(path)))
    }

    #[test]
    fn a_value_is_a_url_a_full_path_or_a_path_in_the_icons_folder() {
        // Arrange
        let config = IconConfig::default();

        // Act / Assert
        assert_eq!(
            resolved("https://example.com/a.png", &config),
            Ok(IconSource::Url("https://example.com/a.png".into()))
        );
        let absolute = std::env::temp_dir().join("a.png");
        assert_eq!(
            resolved(absolute.to_str().unwrap(), &config),
            Ok(IconSource::File(absolute.clone()))
        );
        assert_eq!(resolved("~/Pictures/a.png", &config), file("/home/me/Pictures/a.png"));
        assert_eq!(
            resolved("work/briefcase.svg", &config),
            file("/home/me/.config/flintkey/icons/work/briefcase.svg")
        );
    }

    #[test]
    fn a_relative_path_may_not_leave_the_icons_folder() {
        // Arrange
        let config = IconConfig::default();

        // Act / Assert
        for value in ["../settings.yaml", "work/../../secret.png", "./a.png"] {
            let problem = resolved(value, &config).unwrap_err();
            assert!(problem.contains("may not contain"), "{value}: {problem}");
        }
    }

    #[test]
    fn a_name_from_icons_yaml_is_read_before_a_path() {
        // Arrange
        let config = parse_icons(
            "icons:\n  - name: work\n    path: briefcase.png\n  - name: docs\n    url: https://example.com/docs.ico\n",
        )
        .unwrap();

        // Act / Assert
        assert_eq!(
            resolved("work", &config),
            file("/home/me/.config/flintkey/icons/briefcase.png")
        );
        assert_eq!(
            resolved("docs", &config),
            Ok(IconSource::Url("https://example.com/docs.ico".into()))
        );
        assert!(is_in_icons_dir("work", &config));
        assert!(!is_in_icons_dir("docs", &config));
        assert!(!is_in_icons_dir("~/a.png", &config));
    }

    #[test]
    fn icons_yaml_problems_are_reported_with_their_line() {
        // Arrange
        let source = "icons:\n  - name: a\n  - name: b\n    path: https://example.com/b.png\n  - name: c\n    url: example.com/c.png\n  - name: c2\n    path: c.png\n  - name: c2\n    path: d.png\ntags:\n  web: ''\n";

        // Act
        let problems: Vec<String> = parse_icons(source)
            .unwrap_err()
            .iter()
            .map(ToString::to_string)
            .collect();

        // Assert
        assert_eq!(
            problems,
            [
                "line 2, column 11: icon `a` needs one of `path` or `url`",
                "line 4, column 11: icon `b`: a URL goes in `url`, not `path`",
                "line 6, column 10: icon `c`: `url` must start with https:// or http://",
                "line 9, column 11: icon `c2` is defined twice",
                "line 12, column 8: the icon of tag `web` must not be empty",
            ]
        );
    }

    #[test]
    fn a_command_takes_the_icon_of_its_first_tag_that_has_one() {
        // Arrange
        let config = parse_icons("tags:\n  work: briefcase\n  web: globe.png\n").unwrap();
        let tags = |names: &[&str]| names.iter().map(|name| name.to_string()).collect::<Vec<_>>();

        // Act / Assert
        assert_eq!(tag_icon(&tags(&["misc", "web", "work"]), &config), Some("globe.png"));
        assert_eq!(tag_icon(&tags(&["misc"]), &config), None);
    }

    #[cfg(unix)]
    #[test]
    fn a_link_in_the_icons_folder_may_not_point_outside_it() {
        // Arrange
        let dir = tempfile::tempdir().unwrap();
        let icons = dir.path().join(ICONS_DIR);
        std::fs::create_dir_all(&icons).unwrap();
        std::fs::write(icons.join("inside.png"), "x").unwrap();
        std::fs::write(dir.path().join("secret.txt"), "x").unwrap();
        std::os::unix::fs::symlink(dir.path().join("secret.txt"), icons.join("outside.png")).unwrap();

        // Act / Assert
        assert!(stays_in_icons_dir(&icons.join("inside.png"), dir.path()));
        assert!(!stays_in_icons_dir(&icons.join("outside.png"), dir.path()));
        assert!(!stays_in_icons_dir(&icons.join("missing.png"), dir.path()));
    }
}
