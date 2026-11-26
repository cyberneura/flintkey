//! Hotkey strings (`ctrl+alt+cmd+c`): reading them for comparison, and checking them against the
//! shortcuts macOS itself uses.
//!
//! Registering and validating a hotkey is the desktop app's job (it uses the global shortcut
//! library). The core only needs to know which key combination a string means, to warn when
//! it is one that macOS keeps for itself.

use serde_json::Value;

pub const SHIFT: u64 = 0x2_0000;
pub const CONTROL: u64 = 0x4_0000;
pub const OPTION: u64 = 0x8_0000;
pub const COMMAND: u64 = 0x10_0000;
const MODIFIERS: u64 = SHIFT | CONTROL | OPTION | COMMAND;

/// A key combination: macOS modifier flags and the key's name as written in hotkey strings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Combination {
    pub modifiers: u64,
    pub key: String,
}

/// Reads a hotkey string. `None` if it has no key, more than one key, or an unknown key name.
/// Spellings the global shortcut library accepts are folded together (`Control+KeyC` is `ctrl+c`).
pub fn parse(hotkey: &str) -> Option<Combination> {
    let mut modifiers = 0;
    let mut key = None;
    for token in hotkey.split('+').map(|token| token.trim().to_ascii_lowercase()) {
        let flag = match token.as_str() {
            "ctrl" | "control" => CONTROL,
            "alt" | "option" => OPTION,
            "shift" => SHIFT,
            // The library reads these as Command on macOS. This module only answers questions about macOS.
            "cmd" | "command" | "super" | "cmdorctrl" | "cmdorcontrol" | "commandorctrl" | "commandorcontrol" => {
                COMMAND
            }
            _ => 0,
        };
        if flag != 0 {
            modifiers |= flag;
            continue;
        }
        if key.is_some() {
            return None;
        }
        key = Some(key_alias(&token)?);
    }
    Some(Combination { modifiers, key: key? })
}

fn key_alias(token: &str) -> Option<String> {
    let name = match token {
        "'" => "quote",
        "," => "comma",
        "." => "period",
        "/" => "slash",
        ";" => "semicolon",
        "\\" => "backslash",
        "`" => "backquote",
        "[" => "bracketleft",
        "]" => "bracketright",
        "-" => "minus",
        "=" => "equal",
        "esc" => "escape",
        other => other
            .strip_prefix("key")
            .filter(|rest| rest.len() == 1)
            .or_else(|| other.strip_prefix("digit").filter(|rest| rest.len() == 1))
            .or_else(|| other.strip_prefix("arrow"))
            .unwrap_or(other),
    };
    (mac_key_code(name).is_some() || is_function_key(name)).then(|| name.to_string())
}

fn is_function_key(name: &str) -> bool {
    name.strip_prefix('f')
        .and_then(|number| number.parse::<u8>().ok())
        .is_some_and(|number| (1..=24).contains(&number))
}

/// A shortcut macOS keeps for itself (System Settings → Keyboard → Keyboard Shortcuts).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SystemShortcut {
    pub name: &'static str,
}

/// The macOS shortcuts most likely to collide with a launcher hotkey: (id in `com.apple.symbolichotkeys`,
/// name, default key code, default modifier flags).
const SYSTEM_SHORTCUTS: [(&str, &str, i64, u64); 4] = [
    ("60", "Select the previous input source", 49, CONTROL),
    ("61", "Select next source in Input menu", 49, CONTROL | OPTION),
    ("64", "Show Spotlight search", 49, COMMAND),
    ("65", "Show Finder search window", 49, COMMAND | OPTION),
];

/// The enabled macOS shortcut that `hotkey` would collide with, if any.
///
/// `symbolic_hotkeys` is `~/Library/Preferences/com.apple.symbolichotkeys.plist` as JSON, or `None` if it
/// could not be read. A shortcut missing from it still has its default and is enabled; macOS writes an
/// entry only once the user changes it.
pub fn system_conflict(symbolic_hotkeys: Option<&Value>, hotkey: &str) -> Option<SystemShortcut> {
    let combination = parse(hotkey)?;
    let code = mac_key_code(&combination.key)?;
    let table = symbolic_hotkeys.and_then(|value| value.get("AppleSymbolicHotKeys"));
    SYSTEM_SHORTCUTS
        .iter()
        .find_map(|&(id, name, default_code, default_modifiers)| {
            let entry = table.and_then(|table| table.get(id));
            let enabled = entry
                .and_then(|entry| entry.get("enabled"))
                .map(|enabled| enabled.as_bool().unwrap_or_else(|| enabled.as_i64() == Some(1)))
                .unwrap_or(true);
            let parameters = entry
                .and_then(|entry| entry.pointer("/value/parameters"))
                .and_then(Value::as_array);
            let (shortcut_code, shortcut_modifiers) = match parameters.map(Vec::as_slice) {
                Some([_, code, modifiers]) => (code.as_i64()?, modifiers.as_u64()?),
                _ => (default_code, default_modifiers),
            };
            let same = shortcut_code == code && shortcut_modifiers & MODIFIERS == combination.modifiers;
            (enabled && same).then_some(SystemShortcut { name })
        })
}

/// The macOS virtual key code of a key name. The inverse of [`mac_key_name`].
pub(crate) fn mac_key_code(name: &str) -> Option<i64> {
    (0..=127).find(|&code| mac_key_name(code) == Some(name))
}

/// Names of macOS virtual key codes (positions on the US layout), spelled as the global shortcut library reads them.
pub(crate) fn mac_key_name(code: i64) -> Option<&'static str> {
    Some(match code {
        0 => "a",
        1 => "s",
        2 => "d",
        3 => "f",
        4 => "h",
        5 => "g",
        6 => "z",
        7 => "x",
        8 => "c",
        9 => "v",
        11 => "b",
        12 => "q",
        13 => "w",
        14 => "e",
        15 => "r",
        16 => "y",
        17 => "t",
        18 => "1",
        19 => "2",
        20 => "3",
        21 => "4",
        22 => "6",
        23 => "5",
        24 => "equal",
        25 => "9",
        26 => "7",
        27 => "minus",
        28 => "8",
        29 => "0",
        30 => "bracketright",
        31 => "o",
        32 => "u",
        33 => "bracketleft",
        34 => "i",
        35 => "p",
        36 => "enter",
        37 => "l",
        38 => "j",
        39 => "quote",
        40 => "k",
        41 => "semicolon",
        42 => "backslash",
        43 => "comma",
        44 => "slash",
        45 => "n",
        46 => "m",
        47 => "period",
        48 => "tab",
        49 => "space",
        50 => "backquote",
        51 => "backspace",
        53 => "escape",
        96 => "f5",
        97 => "f6",
        98 => "f7",
        99 => "f3",
        100 => "f8",
        101 => "f9",
        103 => "f11",
        105 => "f13",
        106 => "f16",
        107 => "f14",
        109 => "f10",
        111 => "f12",
        113 => "f15",
        118 => "f4",
        120 => "f2",
        122 => "f1",
        64 => "f17",
        79 => "f18",
        80 => "f19",
        123 => "left",
        124 => "right",
        125 => "down",
        126 => "up",
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn symbolic(entries: Value) -> Value {
        json!({ "AppleSymbolicHotKeys": entries })
    }

    #[test]
    fn spellings_of_the_same_combination_read_the_same() {
        // Arrange
        let spellings = ["ctrl+alt+cmd+c", "Control+Option+Command+KeyC", "super+alt+ctrl+C"];

        for spelling in spellings {
            // Act
            let combination = parse(spelling);

            // Assert
            assert_eq!(
                combination,
                Some(Combination {
                    modifiers: CONTROL | OPTION | COMMAND,
                    key: "c".into()
                }),
                "{spelling}"
            );
        }
        assert_eq!(parse("shift+cmd+'").map(|c| c.key), Some("quote".into()));
        assert_eq!(parse("ctrl+F13").map(|c| c.key), Some("f13".into()));
        assert_eq!(parse("CmdOrCtrl+Space").map(|c| c.modifiers), Some(COMMAND));
    }

    #[test]
    fn strings_without_exactly_one_known_key_are_not_combinations() {
        for hotkey in ["ctrl+alt", "ctrl+a+b", "ctrl+nope", "", "ctrl+f25"] {
            // Act / Assert
            assert_eq!(parse(hotkey), None, "{hotkey}");
        }
    }

    #[test]
    fn default_macos_shortcuts_count_when_the_preferences_do_not_mention_them() {
        // Arrange: a fresh account has no entry for Spotlight, so its default ⌘Space applies.
        let preferences = symbolic(json!({}));

        // Act / Assert
        assert_eq!(
            system_conflict(Some(&preferences), "cmd+space").map(|s| s.name),
            Some("Show Spotlight search")
        );
        assert_eq!(
            system_conflict(None, "ctrl+space").map(|s| s.name),
            Some("Select the previous input source")
        );
        assert_eq!(system_conflict(None, "alt+space"), None);
        assert_eq!(system_conflict(None, "shift+ctrl+space"), None);
    }

    #[test]
    fn disabled_or_moved_shortcuts_do_not_conflict() {
        // Arrange: Spotlight disabled; input source switching moved to ctrl+shift+space.
        let preferences = symbolic(json!({
            "64": { "enabled": false, "value": { "type": "standard", "parameters": [32, 49, 1_048_576] } },
            "60": { "enabled": 1, "value": { "type": "standard", "parameters": [32, 49, 393_216] } },
        }));

        // Act / Assert
        assert_eq!(system_conflict(Some(&preferences), "cmd+space"), None);
        assert_eq!(system_conflict(Some(&preferences), "ctrl+space"), None);
        assert_eq!(
            system_conflict(Some(&preferences), "shift+ctrl+space").map(|s| s.name),
            Some("Select the previous input source")
        );
    }

    #[test]
    fn key_codes_and_names_are_inverse() {
        for code in 0..=127 {
            if let Some(name) = mac_key_name(code) {
                // Act / Assert
                assert_eq!(mac_key_code(name), Some(code), "{name}");
            }
        }
    }
}
