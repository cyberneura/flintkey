//! Alfred のワークフローを Flintkey のコマンドに変換する。
//!
//! Alfred のワークフローは、起点 (キーワード、ホットキー) からアクション (URL を開く、スクリプト、…) へ
//! 線でつないだグラフ。Flintkey のコマンドは「起点 1 つ (キーワードとホットキー) → アクション 1 つ」なので、
//! その形に収まるものだけを変換し、収まらないものは理由を付けて返す。
//!
//! 入力は `info.plist` を JSON にしたもの (`plutil -convert json`)。ファイルは読まない。

use crate::command::{Action, Command, Os};
use serde_json::Value;
use std::collections::{BTreeMap, HashMap, HashSet};

/// ワークフロー 1 つ。
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub disabled: bool,
    objects: Vec<Object>,
    /// 起点の uid → 修飾キー無しでつながる先の uid。
    connections: HashMap<String, Vec<String>>,
    /// 修飾キーを押しながら実行したときだけつながる先がある uid。
    alternatives: HashSet<String>,
    /// ワークフローの変数 (設定画面で定義するものを含む) を持つか。スクリプトに渡らなくなる。
    has_variables: bool,
}

#[derive(Debug, Clone, PartialEq)]
struct Object {
    uid: String,
    /// `alfred.workflow.` を除いた種類 (`input.keyword`、`action.openurl` など)。
    kind: String,
    config: Value,
}

/// 変換できたコマンド。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Imported {
    pub id: String,
    pub command: Command,
    /// どこから来たか (`<ワークフロー名>: <キーワードかホットキー>`)。
    pub origin: String,
    /// 変換はしたが、確かめてほしいこと。
    pub warnings: Vec<String>,
}

/// 変換しなかった起点。
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Skipped {
    /// どこのものか (`<ワークフロー名>: <キーワードかホットキー>`)。
    pub origin: String,
    pub reason: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
pub struct Conversion {
    pub commands: Vec<Imported>,
    pub skipped: Vec<Skipped>,
}

impl Workflow {
    /// `info.plist` を JSON にしたものを読む。
    pub fn from_json(info: &Value) -> Result<Workflow, String> {
        let name = text(info, "name").ok_or("the workflow has no name")?.to_string();
        let objects = info
            .get("objects")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|object| {
                Some(Object {
                    uid: text(object, "uid")?.to_string(),
                    kind: text(object, "type")?.trim_start_matches("alfred.workflow.").to_string(),
                    config: object.get("config").cloned().unwrap_or(Value::Null),
                })
            })
            .collect();
        let mut connections: HashMap<String, Vec<String>> = HashMap::new();
        let mut alternatives = HashSet::new();
        for (from, links) in info.get("connections").and_then(Value::as_object).into_iter().flatten() {
            for link in links.as_array().map(Vec::as_slice).unwrap_or_default() {
                let Some(to) = text(link, "destinationuid") else {
                    continue;
                };
                if link.get("modifiers").and_then(Value::as_u64).unwrap_or(0) == 0 {
                    connections.entry(from.clone()).or_default().push(to.to_string());
                } else {
                    alternatives.insert(from.clone());
                }
            }
        }
        Ok(Workflow {
            name,
            disabled: info.get("disabled").and_then(Value::as_bool).unwrap_or(false),
            objects,
            connections,
            alternatives,
            has_variables: ["variables", "userconfigurationconfig"].iter().any(|key| {
                info.get(*key).is_some_and(|value| {
                    value.as_object().is_some_and(|map| !map.is_empty())
                        || value.as_array().is_some_and(|list| !list.is_empty())
                })
            }),
        })
    }

    fn object(&self, uid: &str) -> Option<&Object> {
        self.objects.iter().find(|object| object.uid == uid)
    }

    fn targets(&self, uid: &str) -> Vec<&Object> {
        self.connections
            .get(uid)
            .map(Vec::as_slice)
            .unwrap_or_default()
            .iter()
            .filter_map(|to| self.object(to))
            .collect()
    }
}

fn text<'a>(value: &'a Value, key: &str) -> Option<&'a str> {
    value.get(key).and_then(Value::as_str)
}

fn config_text<'a>(object: &'a Object, key: &str) -> &'a str {
    text(&object.config, key).unwrap_or_default()
}

/// 起点の種類。
enum Trigger {
    Keyword { keywords: Vec<String>, title: String },
    Hotkey(String),
}

/// 変換の途中の 1 件。id はまだ決まっていない。
struct Draft {
    workflow: usize,
    target: String,
    trigger_label: String,
    keywords: Vec<String>,
    title: Option<String>,
    hotkey: Option<String>,
    action: Action,
    macos_only: bool,
    warnings: Vec<String>,
}

/// ワークフローをコマンドに変換する。結果はワークフロー名の順で、何度変換しても同じ id になる。
pub fn convert(workflows: &[Workflow]) -> Conversion {
    let mut order: Vec<usize> = (0..workflows.len()).collect();
    order.sort_by(|&a, &b| workflows[a].name.cmp(&workflows[b].name));

    let mut conversion = Conversion::default();
    let mut drafts = Vec::new();
    for index in order {
        let workflow = &workflows[index];
        if workflow.disabled {
            conversion.skipped.push(Skipped {
                origin: workflow.name.clone(),
                reason: "the workflow is disabled".into(),
            });
            continue;
        }
        for object in &workflow.objects {
            let origin = |label: &str| format!("{}: {label}", workflow.name);
            let (trigger, label) = match read_trigger(object) {
                Ok(Some(found)) => found,
                Ok(None) => continue,
                Err((label, reason)) => {
                    conversion.skipped.push(Skipped {
                        origin: origin(&label),
                        reason,
                    });
                    continue;
                }
            };
            let skip = |reason: String| Skipped {
                origin: origin(&label),
                reason,
            };
            let targets = workflow.targets(&object.uid);
            let target = match targets.as_slice() {
                [] => {
                    conversion.skipped.push(skip("it is not connected to an action".into()));
                    continue;
                }
                [one] => *one,
                many => {
                    conversion.skipped.push(skip(format!(
                        "it runs {} actions; a Flintkey command runs one",
                        many.len()
                    )));
                    continue;
                }
            };
            if !workflow.targets(&target.uid).is_empty() {
                conversion.skipped.push(skip(format!(
                    "its `{}` passes the result on to further steps",
                    target.kind
                )));
                continue;
            }
            let converted = match convert_action(target) {
                Ok(converted) => converted,
                Err(reason) => {
                    conversion.skipped.push(skip(reason));
                    continue;
                }
            };
            let mut warnings = converted.warnings;
            if workflow.has_variables && matches!(converted.action, Action::Script { .. } | Action::Terminal { .. }) {
                warnings.push("the workflow defines variables, which the script will not receive".into());
            }
            if workflow.alternatives.contains(&object.uid) {
                warnings.push("the variants run with a modifier key held were not imported".into());
            }
            let (keywords, title, hotkey) = match trigger {
                Trigger::Keyword { keywords, title } => (keywords, Some(title), None),
                Trigger::Hotkey(hotkey) => (Vec::new(), None, Some(hotkey)),
            };
            drafts.push(Draft {
                workflow: index,
                target: target.uid.clone(),
                trigger_label: label,
                keywords,
                title,
                hotkey,
                action: converted.action,
                macos_only: converted.macos_only,
                warnings,
            });
        }
    }

    let drafts = merge_hotkeys_into_keywords(drafts);
    let mut taken = HashSet::new();
    for draft in drafts {
        let workflow = &workflows[draft.workflow];
        let base = draft
            .keywords
            .first()
            .cloned()
            .or_else(|| app_name(&draft.action))
            .or_else(|| draft.hotkey.as_ref().map(|hotkey| format!("hotkey {hotkey}")))
            .unwrap_or_else(|| "alfred".into());
        let id = unique_id(&base, &mut taken);
        let title = draft
            .title
            .or_else(|| app_name(&draft.action))
            .unwrap_or_else(|| format!("{} {}", workflow.name, draft.hotkey.clone().unwrap_or_default()));
        conversion.commands.push(Imported {
            id,
            command: Command {
                title,
                keywords: draft.keywords,
                hotkey: draft.hotkey,
                pin: false,
                tags: vec![workflow.name.clone()],
                os: if draft.macos_only {
                    [Os::Macos].into()
                } else {
                    [].into()
                },
                action: draft.action,
            },
            origin: format!("{}: {}", workflow.name, draft.trigger_label),
            warnings: draft.warnings,
        });
    }
    conversion
}

/// 同じアクションにキーワード 1 つとホットキー 1 つがつながっていれば、1 つのコマンドにする。
/// それ以外の組み合わせは別々のコマンドのまま (どのキーワードにホットキーを付けるか決められない)。
fn merge_hotkeys_into_keywords(drafts: Vec<Draft>) -> Vec<Draft> {
    let mut by_target: BTreeMap<(usize, String), (Vec<usize>, Vec<usize>)> = BTreeMap::new();
    for (index, draft) in drafts.iter().enumerate() {
        let entry = by_target.entry((draft.workflow, draft.target.clone())).or_default();
        if draft.hotkey.is_some() {
            entry.1.push(index);
        } else {
            entry.0.push(index);
        }
    }
    let mut absorbed: HashMap<usize, usize> = HashMap::new();
    for (keywords, hotkeys) in by_target.values() {
        if let ([keyword], [hotkey]) = (keywords.as_slice(), hotkeys.as_slice()) {
            absorbed.insert(*hotkey, *keyword);
        }
    }
    let mut hotkeys_for: HashMap<usize, (String, String)> = HashMap::new();
    for (&hotkey, &keyword) in &absorbed {
        let draft = &drafts[hotkey];
        hotkeys_for.insert(
            keyword,
            (draft.hotkey.clone().unwrap_or_default(), draft.trigger_label.clone()),
        );
    }
    drafts
        .into_iter()
        .enumerate()
        .filter(|(index, _)| !absorbed.contains_key(index))
        .map(|(index, mut draft)| {
            if let Some((hotkey, label)) = hotkeys_for.remove(&index) {
                draft.hotkey = Some(hotkey);
                draft.trigger_label = format!("{} + {label}", draft.trigger_label);
            }
            draft
        })
        .collect()
}

/// 起点を読む。起点でなければ `Ok(None)`。変換できない起点は `Err((表示名, 理由))`。
fn read_trigger(object: &Object) -> Result<Option<(Trigger, String)>, (String, String)> {
    match object.kind.as_str() {
        "input.keyword" => {
            let raw = config_text(object, "keyword");
            let mut keywords: Vec<String> = Vec::new();
            for keyword in raw.split("||").map(str::trim).filter(|keyword| !keyword.is_empty()) {
                if !keywords.iter().any(|known| known == keyword) {
                    keywords.push(keyword.to_string());
                }
            }
            let label = format!("keyword `{raw}`");
            let Some(first) = keywords.first() else {
                return Err((label, "the keyword is empty".into()));
            };
            let title = match config_text(object, "text").trim() {
                "" => first.clone(),
                text => text.to_string(),
            };
            Ok(Some((Trigger::Keyword { keywords, title }, label)))
        }
        "trigger.hotkey" => {
            let label = format!("hotkey `{}`", config_text(object, "hotstring"));
            let hotkey = read_hotkey(&object.config).map_err(|reason| (label.clone(), reason))?;
            Ok(Some((Trigger::Hotkey(hotkey.clone()), format!("hotkey `{hotkey}`"))))
        }
        kind if kind.starts_with("input.") || kind.starts_with("trigger.") => {
            Err((format!("`{kind}`"), format!("Flintkey has no trigger like `{kind}`")))
        }
        _ => Ok(None),
    }
}

const SHIFT: u64 = 0x2_0000;
const CONTROL: u64 = 0x4_0000;
const OPTION: u64 = 0x8_0000;
const COMMAND: u64 = 0x10_0000;

/// Alfred のホットキー (macOS の修飾フラグと仮想キーコード) を Flintkey の書式にする。
fn read_hotkey(config: &Value) -> Result<String, String> {
    let modifiers = config.get("hotmod").and_then(Value::as_u64).unwrap_or(0);
    let code = config.get("hotkey").and_then(Value::as_i64).unwrap_or(-1);
    if config.get("modsmode").and_then(Value::as_u64).unwrap_or(0) != 0 {
        return Err("it fires on a modifier key alone, which Flintkey cannot register".into());
    }
    if modifiers & !(SHIFT | CONTROL | OPTION | COMMAND) != 0 {
        return Err(format!(
            "it uses modifier flags {modifiers:#x}, which Flintkey cannot express"
        ));
    }
    if modifiers == 0 {
        return Err("it has no modifier key".into());
    }
    let related = config
        .get("relatedApps")
        .and_then(Value::as_array)
        .is_some_and(|apps| !apps.is_empty());
    if related && config.get("relatedAppsMode").and_then(Value::as_u64).unwrap_or(0) != 0 {
        return Err("it is limited to particular applications, which Flintkey hotkeys cannot be".into());
    }
    let key = key_name(code).ok_or_else(|| format!("key code {code} has no name in Flintkey"))?;
    let mut parts: Vec<&str> = Vec::new();
    for (flag, name) in [(CONTROL, "ctrl"), (OPTION, "alt"), (SHIFT, "shift"), (COMMAND, "cmd")] {
        if modifiers & flag != 0 {
            parts.push(name);
        }
    }
    parts.push(key);
    Ok(parts.join("+"))
}

/// 移行が書き出すことのあるキーの名前。デスクトップアプリが、すべて登録できる名前であることを確かめる。
pub fn key_names() -> impl Iterator<Item = &'static str> {
    (0..=127).filter_map(key_name)
}

/// macOS の仮想キーコード (US 配列での位置) の名前。グローバルショートカットのライブラリが読める名前にする。
fn key_name(code: i64) -> Option<&'static str> {
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

struct Converted {
    action: Action,
    macos_only: bool,
    warnings: Vec<String>,
}

fn convert_action(object: &Object) -> Result<Converted, String> {
    let script = |run: String| Action::Script {
        run,
        cwd: None,
        env: BTreeMap::new(),
    };
    let mac = |action: Action| Converted {
        action,
        macos_only: true,
        warnings: Vec::new(),
    };
    let converted = match object.kind.as_str() {
        "action.openurl" => {
            let url = config_text(object, "url").trim();
            refuse_placeholders(url)?;
            let browser = config_text(object, "browser").trim();
            if !browser.is_empty() {
                mac(script(format!("open -b {} {}", shell_word(browser), shell_word(url))))
            } else if crate::command::has_url_scheme(url) {
                Converted {
                    action: Action::Url { url: url.to_string() },
                    macos_only: false,
                    warnings: Vec::new(),
                }
            } else {
                // URL そのものは出さない。トークンを含むことがある。
                return Err("the URL has no scheme such as https://".into());
            }
        }
        "action.launchfiles" => match object.config.get("paths").and_then(Value::as_array).map(Vec::as_slice) {
            Some([path]) => {
                let path = path.as_str().unwrap_or_default().trim();
                if path.is_empty() {
                    return Err("it opens an empty path".into());
                }
                // `type: app` は `open -a` で起動するので、アプリ以外 (フォルダ、書類) は開けない。
                if path.trim_end_matches('/').ends_with(".app") {
                    let mut converted = mac(Action::App {
                        app: path.to_string(),
                        args: Vec::new(),
                    });
                    if object.config.get("toggle").and_then(Value::as_bool) == Some(true) {
                        converted.warnings.push(
                            "Alfred hid the app when it was already in front; Flintkey only brings it to the front"
                                .into(),
                        );
                    }
                    converted
                } else {
                    mac(script(format!("open {}", shell_path(path))))
                }
            }
            Some(paths) if paths.len() > 1 => {
                return Err(format!(
                    "it opens {} files at once; a Flintkey command opens one",
                    paths.len()
                ));
            }
            _ => return Err("it opens no file".into()),
        },
        "action.openfile" => {
            let path = config_text(object, "sourcefile").trim();
            if path.is_empty() {
                return Err("it opens the file passed in, which Flintkey cannot pass".into());
            }
            refuse_placeholders(path)?;
            let with = config_text(object, "openwith").trim();
            let run = if with.is_empty() {
                format!("open {}", shell_path(path))
            } else {
                format!("open -a {} {}", shell_path(with), shell_path(path))
            };
            mac(script(run))
        }
        "action.script" => {
            if !config_text(object, "scriptfile").is_empty() {
                return Err("it runs a script file inside the workflow folder".into());
            }
            let source = config_text(object, "script");
            refuse_arguments(source)?;
            let language = object.config.get("type").and_then(Value::as_u64).unwrap_or(u64::MAX);
            let run = match language {
                // zsh。Flintkey はスクリプトをログインシェルで実行する。
                5 | 11 => source.to_string(),
                0 => heredoc("/bin/bash", source),
                6 => heredoc("osascript", source),
                7 => heredoc("osascript -l JavaScript", source),
                other => return Err(format!("scripts of language #{other} are not converted")),
            };
            let mut converted = mac(script(run));
            converted.warnings = secret_warnings(source);
            converted
        }
        "action.applescript" => {
            let source = config_text(object, "applescript");
            refuse_arguments(source)?;
            let mut converted = mac(script(heredoc("osascript", source)));
            converted.warnings = secret_warnings(source);
            converted
        }
        "action.terminalcommand" => {
            let source = config_text(object, "script");
            refuse_arguments(source)?;
            if source.trim().is_empty() {
                return Err("the terminal command is empty".into());
            }
            let mut converted = mac(Action::Terminal {
                run: source.to_string(),
                cwd: None,
            });
            converted.warnings = secret_warnings(source);
            converted
        }
        "action.browseinterminal" => {
            let path = config_text(object, "path").trim();
            if path.is_empty() {
                return Err("it browses the path passed in, which Flintkey cannot pass".into());
            }
            refuse_placeholders(path)?;
            mac(Action::Terminal {
                run: format!("cd {}", shell_path(path)),
                cwd: None,
            })
        }
        kind => return Err(format!("Flintkey has no action like `{kind}`")),
    };
    Ok(converted)
}

/// Alfred が実行時に置き換える `{query}` などを含むなら変換しない。そのまま渡すと、文字どおりの
/// `{query}` を開いてしまう。
fn refuse_placeholders(text: &str) -> Result<(), String> {
    const NAMES: [&str; 9] = [
        "query",
        "clipboard",
        "date",
        "time",
        "datetime",
        "random",
        "uuid",
        "cursor",
        "selection",
    ];
    let mut rest = text;
    while let Some(start) = rest.find('{') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('}') else { break };
        let name = &after[..end];
        let base = name.split(':').next().unwrap_or_default();
        if NAMES.contains(&base) || base == "var" || base == "snippet" {
            return Err(format!(
                "it uses Alfred's `{{{name}}}`; arguments are not supported yet"
            ));
        }
        rest = &after[end + 1..];
    }
    Ok(())
}

/// スクリプトが Alfred から渡される引数を使うなら変換しない。
fn refuse_arguments(source: &str) -> Result<(), String> {
    refuse_placeholders(source)?;
    for marker in ["$1", "${1", "$@", "\"$*\"", "argv"] {
        if source.contains(marker) {
            return Err(format!(
                "the script reads its argument (`{marker}`); arguments are not supported yet"
            ));
        }
    }
    Ok(())
}

/// 秘密情報を含んでいそうな語。設定フォルダを Git で共有する前に確かめてもらう。
fn secret_warnings(source: &str) -> Vec<String> {
    const WORDS: [&str; 8] = [
        "token",
        "secret",
        "password",
        "passwd",
        "api_key",
        "apikey",
        "private_key",
        ".env",
    ];
    let lowered = source.to_lowercase();
    WORDS
        .iter()
        .filter(|word| lowered.contains(*word))
        .map(|word| format!("the script mentions `{word}`; make sure it holds no secret before sharing the folder"))
        .collect()
}

/// `interpreter` に標準入力からスクリプトを渡すシェルのコマンド。
fn heredoc(interpreter: &str, source: &str) -> String {
    let mut marker = "FLINTKEY_SCRIPT".to_string();
    while source.lines().any(|line| line.trim() == marker) {
        marker.push('_');
    }
    let body = source.strip_suffix('\n').unwrap_or(source);
    format!("{interpreter} <<'{marker}'\n{body}\n{marker}\n")
}

/// シェルの 1 語。引用符が要らなければそのまま。
fn shell_word(text: &str) -> String {
    let plain = !text.is_empty()
        && text.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.' | '/' | ':' | '@' | '%' | '+' | ',' | '=')
        });
    if plain {
        text.to_string()
    } else {
        format!("'{}'", text.replace('\'', r"'\''"))
    }
}

/// シェルに渡すパス。先頭の `~/` は展開されるように引用符の外に出す。
fn shell_path(path: &str) -> String {
    match path.strip_prefix("~/") {
        Some(rest) if !rest.is_empty() => format!("~/{}", shell_word(rest)),
        _ if path == "~" || path == "~/" => "~".to_string(),
        _ => shell_word(path),
    }
}

/// アプリを開くアクションなら、アプリの名前 (`/Applications/Google Chrome.app` → `Google Chrome`)。
fn app_name(action: &Action) -> Option<String> {
    let Action::App { app, .. } = action else {
        return None;
    };
    let file = app.trim_end_matches('/').rsplit('/').next()?;
    let name = file.strip_suffix(".app").unwrap_or(file);
    (!name.is_empty()).then(|| name.to_string())
}

/// ファイル名に使える id。macOS の既定のファイルシステムは大文字と小文字を区別しないので、小文字にそろえる。
fn unique_id(base: &str, taken: &mut HashSet<String>) -> String {
    let mut cleaned = String::new();
    for c in base.trim().chars() {
        let c = if c.is_whitespace() || matches!(c, '/' | '\\' | ':' | '\0' | '|') {
            '-'
        } else {
            c
        };
        if c == '-' && (cleaned.is_empty() || cleaned.ends_with('-')) {
            continue;
        }
        cleaned.extend(c.to_lowercase());
    }
    let cleaned = cleaned.trim_start_matches('.').trim_end_matches('-').to_string();
    let base = if cleaned.is_empty() {
        "alfred".to_string()
    } else {
        cleaned
    };
    let mut id = base.clone();
    let mut counter = 2;
    while !taken.insert(id.clone()) {
        id = format!("{base}-{counter}");
        counter += 1;
    }
    id
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::command::parse_command;
    use crate::document::render;
    use serde_json::json;

    fn object(uid: &str, kind: &str, config: Value) -> Value {
        json!({ "uid": uid, "type": format!("alfred.workflow.{kind}"), "config": config })
    }

    fn keyword(uid: &str, keyword: &str, text: &str) -> Value {
        object(
            uid,
            "input.keyword",
            json!({ "keyword": keyword, "text": text, "argumenttype": 2 }),
        )
    }

    fn hotkey(uid: &str, modifiers: u64, code: i64, label: &str) -> Value {
        object(
            uid,
            "trigger.hotkey",
            json!({ "hotmod": modifiers, "hotkey": code, "hotstring": label, "modsmode": 0 }),
        )
    }

    fn workflow(name: &str, objects: Vec<Value>, links: &[(&str, &str)]) -> Workflow {
        let mut connections = serde_json::Map::new();
        for (from, to) in links {
            connections
                .entry(from.to_string())
                .or_insert_with(|| json!([]))
                .as_array_mut()
                .unwrap()
                .push(json!({ "destinationuid": to, "modifiers": 0 }));
        }
        Workflow::from_json(&json!({ "name": name, "disabled": false, "objects": objects, "connections": connections }))
            .unwrap()
    }

    fn reasons(conversion: &Conversion) -> Vec<String> {
        conversion
            .skipped
            .iter()
            .map(|skipped| format!("{} — {}", skipped.origin, skipped.reason))
            .collect()
    }

    const CTRL_ALT_CMD: u64 = 0x1C_0000;

    #[test]
    fn keyword_opening_a_url_becomes_a_url_command_tagged_with_the_workflow() {
        // Arrange
        let workflows = [workflow(
            "TORICO/URL",
            vec![
                keyword("k", "jira||チケット", "Jira を開く"),
                object(
                    "a",
                    "action.openurl",
                    json!({ "url": "https://example.atlassian.net/" }),
                ),
            ],
            &[("k", "a")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        assert_eq!(reasons(&conversion), Vec::<String>::new());
        assert_eq!(
            conversion.commands,
            [Imported {
                id: "jira".into(),
                command: Command {
                    title: "Jira を開く".into(),
                    keywords: vec!["jira".into(), "チケット".into()],
                    hotkey: None,
                    pin: false,
                    tags: vec!["TORICO/URL".into()],
                    os: [].into(),
                    action: Action::Url {
                        url: "https://example.atlassian.net/".into()
                    },
                },
                origin: "TORICO/URL: keyword `jira||チケット`".into(),
                warnings: vec![],
            }]
        );
    }

    #[test]
    fn hotkey_opening_an_app_becomes_a_macos_app_command_named_after_the_app() {
        // Arrange: ctrl+alt+cmd+C (キーコード 8) で Chrome を開く。
        let workflows = [workflow(
            "アプリ切り替えホットキー",
            vec![
                hotkey("h", CTRL_ALT_CMD, 8, "C"),
                object(
                    "a",
                    "action.launchfiles",
                    json!({ "paths": ["/Applications/Google Chrome.app"], "toggle": false }),
                ),
            ],
            &[("h", "a")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        assert_eq!(reasons(&conversion), Vec::<String>::new());
        let imported = &conversion.commands[0];
        assert_eq!(imported.id, "google-chrome");
        assert_eq!(imported.command.title, "Google Chrome");
        assert_eq!(imported.command.hotkey.as_deref(), Some("ctrl+alt+cmd+c"));
        assert_eq!(imported.command.os, [Os::Macos].into());
        assert_eq!(
            imported.command.action,
            Action::App {
                app: "/Applications/Google Chrome.app".into(),
                args: vec![]
            }
        );
    }

    #[test]
    fn launching_a_folder_or_document_opens_it_instead_of_running_it_as_an_app() {
        // Arrange
        let workflows = [workflow(
            "W",
            vec![
                keyword("d", "downloads", "Downloads"),
                object("a", "action.launchfiles", json!({ "paths": ["~/Downloads"] })),
                keyword("p", "manual", "Manual"),
                object(
                    "b",
                    "action.launchfiles",
                    json!({ "paths": ["/Users/me/Documents/Manual 2.pdf"] }),
                ),
            ],
            &[("d", "a"), ("p", "b")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        let action = |id: &str| {
            conversion
                .commands
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .command
                .action
                .clone()
        };
        let script = |run: &str| Action::Script {
            run: run.into(),
            cwd: None,
            env: BTreeMap::new(),
        };
        assert_eq!(action("downloads"), script("open ~/Downloads"));
        assert_eq!(action("manual"), script("open '/Users/me/Documents/Manual 2.pdf'"));
    }

    #[test]
    fn hotkey_limited_to_some_applications_is_skipped() {
        // Act
        let limited = read_hotkey(&json!({
            "hotmod": CTRL_ALT_CMD, "hotkey": 8, "relatedAppsMode": 1, "relatedApps": ["com.apple.finder"]
        }));
        let everywhere = read_hotkey(&json!({
            "hotmod": CTRL_ALT_CMD, "hotkey": 8, "relatedAppsMode": 0, "relatedApps": []
        }));

        // Assert
        assert!(limited.is_err(), "{limited:?}");
        assert_eq!(everywhere.as_deref(), Ok("ctrl+alt+cmd+c"));
    }

    #[test]
    fn scripts_of_a_workflow_with_variables_and_toggling_apps_carry_warnings() {
        // Arrange
        let info = json!({
            "name": "W",
            "variables": { "API_HOST": "example.com" },
            "objects": [
                keyword("k", "run", "Run"),
                object("s", "action.script", json!({ "script": "curl \"$API_HOST\"", "type": 5 })),
                keyword("u", "site", "Site"),
                object("o", "action.openurl", json!({ "url": "https://example.com" })),
                keyword("a", "app", "App"),
                object("l", "action.launchfiles", json!({ "paths": ["/Applications/A.app"], "toggle": true })),
            ],
            "connections": {
                "k": [{ "destinationuid": "s", "modifiers": 0 }],
                "u": [{ "destinationuid": "o", "modifiers": 0 }],
                "a": [{ "destinationuid": "l", "modifiers": 0 }],
            },
        });

        // Act
        let conversion = convert(&[Workflow::from_json(&info).unwrap()]);

        // Assert
        let warnings = |id: &str| {
            conversion
                .commands
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .warnings
                .clone()
        };
        assert_eq!(
            warnings("run"),
            ["the workflow defines variables, which the script will not receive"]
        );
        assert!(warnings("site").is_empty());
        assert_eq!(
            warnings("app"),
            ["Alfred hid the app when it was already in front; Flintkey only brings it to the front"]
        );
    }

    #[test]
    fn modifiers_and_keys_are_named_like_the_shortcut_library_expects() {
        let cases = [
            (0x18_0000, 49, "alt+cmd+space"),
            (0x12_0000, 39, "shift+cmd+quote"),
            (0x1E_0000, 18, "ctrl+alt+shift+cmd+1"),
            (0x0C_0000, 122, "ctrl+alt+f1"),
        ];
        for (modifiers, code, expected) in cases {
            // Act
            let hotkey = read_hotkey(&json!({ "hotmod": modifiers, "hotkey": code }));

            // Assert
            assert_eq!(hotkey.as_deref(), Ok(expected), "{modifiers:#x} {code}");
        }
    }

    #[test]
    fn hotkeys_flintkey_cannot_register_are_skipped_with_a_reason() {
        // Arrange: 修飾キー無し、fn キー (0x800000)、名前の無いキーコード、修飾キー単体。
        let cases = [
            json!({ "hotmod": 0, "hotkey": 8 }),
            json!({ "hotmod": 0x80_0000 | CTRL_ALT_CMD, "hotkey": 8 }),
            json!({ "hotmod": CTRL_ALT_CMD, "hotkey": 999 }),
            json!({ "hotmod": CTRL_ALT_CMD, "hotkey": 8, "modsmode": 2 }),
        ];

        for config in cases {
            // Act
            let hotkey = read_hotkey(&config);

            // Assert
            assert!(hotkey.is_err(), "{config} gave {hotkey:?}");
        }
    }

    #[test]
    fn keyword_and_hotkey_on_the_same_action_become_one_command() {
        // Arrange
        let workflows = [workflow(
            "アプリエイリアス",
            vec![
                keyword("k", "claude", "Claude"),
                hotkey("h", 0x14_0000, 49, "Space"),
                object(
                    "a",
                    "action.launchfiles",
                    json!({ "paths": ["/Applications/Claude.app"] }),
                ),
            ],
            &[("k", "a"), ("h", "a")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        assert_eq!(conversion.commands.len(), 1, "{:?}", conversion.commands);
        let command = &conversion.commands[0].command;
        assert_eq!(command.keywords, ["claude"]);
        assert_eq!(command.hotkey.as_deref(), Some("ctrl+cmd+space"));
        assert_eq!(
            conversion.commands[0].origin,
            "アプリエイリアス: keyword `claude` + hotkey `ctrl+cmd+space`"
        );
    }

    #[test]
    fn two_keywords_on_one_action_keep_the_hotkey_on_its_own_command() {
        // Arrange: どちらのキーワードにホットキーを付けるかは決められない。
        let workflows = [workflow(
            "W",
            vec![
                keyword("k1", "one", "One"),
                keyword("k2", "two", "Two"),
                hotkey("h", CTRL_ALT_CMD, 0, "A"),
                object("a", "action.launchfiles", json!({ "paths": ["/Applications/A.app"] })),
            ],
            &[("k1", "a"), ("k2", "a"), ("h", "a")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        let ids: Vec<&str> = conversion.commands.iter().map(|c| c.id.as_str()).collect();
        assert_eq!(ids, ["one", "two", "a"]);
        assert!(conversion.commands[..2].iter().all(|c| c.command.hotkey.is_none()));
    }

    #[test]
    fn what_does_not_fit_one_trigger_and_one_action_is_skipped_with_a_reason() {
        // Arrange
        let workflows = [workflow(
            "W",
            vec![
                keyword("query", "search", "Search"),
                object(
                    "search-url",
                    "action.openurl",
                    json!({ "url": "https://example.com/?q={query}" }),
                ),
                keyword("two", "both", "Both"),
                object("u1", "action.openurl", json!({ "url": "https://a.example" })),
                object("u2", "action.openurl", json!({ "url": "https://b.example" })),
                keyword("copy", "mail", "Mail"),
                object("clip", "output.clipboard", json!({ "clipboardtext": "me@example.com" })),
                keyword("chain", "chain", "Chain"),
                object("s", "action.script", json!({ "script": "date", "type": 5 })),
                object("n", "output.notification", json!({})),
                keyword("lonely", "lonely", "Lonely"),
                object("f", "input.scriptfilter", json!({})),
                keyword("arg", "arg", "Arg"),
                object("s2", "action.script", json!({ "script": "echo \"$1\"", "type": 5 })),
            ],
            &[
                ("query", "search-url"),
                ("two", "u1"),
                ("two", "u2"),
                ("copy", "clip"),
                ("chain", "s"),
                ("s", "n"),
                ("arg", "s2"),
            ],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        assert!(conversion.commands.is_empty(), "{:?}", conversion.commands);
        assert_eq!(
            reasons(&conversion),
            [
                "W: keyword `search` — it uses Alfred's `{query}`; arguments are not supported yet",
                "W: keyword `both` — it runs 2 actions; a Flintkey command runs one",
                "W: keyword `mail` — Flintkey has no action like `output.clipboard`",
                "W: keyword `chain` — its `action.script` passes the result on to further steps",
                "W: keyword `lonely` — it is not connected to an action",
                "W: `input.scriptfilter` — Flintkey has no trigger like `input.scriptfilter`",
                "W: keyword `arg` — the script reads its argument (`$1`); arguments are not supported yet",
            ]
        );
    }

    #[test]
    fn scripts_are_converted_by_language_and_mentioning_secrets_is_flagged() {
        // Arrange
        let workflows = [workflow(
            "W",
            vec![
                keyword("z", "zsh", "Zsh"),
                object(
                    "zs",
                    "action.script",
                    json!({ "script": "source ~/.env\nopen \"x://y\"", "type": 5 }),
                ),
                keyword("j", "jxa", "JXA"),
                object(
                    "js",
                    "action.script",
                    json!({ "script": "function run() {\n  return 1\n}", "type": 7 }),
                ),
                keyword("a", "as", "AppleScript"),
                object(
                    "ap",
                    "action.applescript",
                    json!({ "applescript": "tell application \"Finder\" to activate" }),
                ),
                keyword("t", "term", "Terminal"),
                object("tc", "action.terminalcommand", json!({ "script": "ssh host" })),
                keyword("p", "php", "PHP"),
                object("ph", "action.script", json!({ "script": "<?php echo 1;", "type": 1 })),
            ],
            &[("z", "zs"), ("j", "js"), ("a", "ap"), ("t", "tc"), ("p", "ph")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        let by_id = |id: &str| conversion.commands.iter().find(|c| c.id == id).unwrap();
        assert_eq!(
            by_id("zsh").command.action,
            Action::Script {
                run: "source ~/.env\nopen \"x://y\"".into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
        assert_eq!(
            by_id("zsh").warnings,
            ["the script mentions `.env`; make sure it holds no secret before sharing the folder"]
        );
        assert_eq!(
            by_id("jxa").command.action,
            Action::Script {
                run: "osascript -l JavaScript <<'FLINTKEY_SCRIPT'\nfunction run() {\n  return 1\n}\nFLINTKEY_SCRIPT\n"
                    .into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
        assert_eq!(
            by_id("as").command.action,
            Action::Script {
                run: "osascript <<'FLINTKEY_SCRIPT'\ntell application \"Finder\" to activate\nFLINTKEY_SCRIPT\n".into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
        assert_eq!(
            by_id("term").command.action,
            Action::Terminal {
                run: "ssh host".into(),
                cwd: None
            }
        );
        assert!(by_id("term").warnings.is_empty());
        assert_eq!(
            reasons(&conversion),
            ["W: keyword `php` — scripts of language #1 are not converted"]
        );
        assert!(conversion.commands.iter().all(|c| c.command.os == [Os::Macos].into()));
    }

    #[test]
    fn heredoc_marker_never_collides_with_a_line_of_the_script() {
        // Act
        let run = heredoc("osascript", "FLINTKEY_SCRIPT\nsay 1\n");

        // Assert
        assert_eq!(
            run,
            "osascript <<'FLINTKEY_SCRIPT_'\nFLINTKEY_SCRIPT\nsay 1\nFLINTKEY_SCRIPT_\n"
        );
    }

    #[test]
    fn url_for_a_given_browser_and_folders_open_through_the_shell() {
        // Arrange
        let workflows = [workflow(
            "W",
            vec![
                keyword("f", "firefox", "In Firefox"),
                object(
                    "u",
                    "action.openurl",
                    json!({ "url": "https://example.com/a b?x='1'", "browser": "org.mozilla.firefox" }),
                ),
                keyword("d", "desk", "Desktop"),
                object(
                    "o",
                    "action.openfile",
                    json!({ "sourcefile": "~/Desktop/my shots", "openwith": "" }),
                ),
                keyword("b", "browse", "Browse"),
                object("t", "action.browseinterminal", json!({ "path": "~/workspace" })),
            ],
            &[("f", "u"), ("d", "o"), ("b", "t")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        let action = |id: &str| {
            conversion
                .commands
                .iter()
                .find(|c| c.id == id)
                .unwrap()
                .command
                .action
                .clone()
        };
        assert_eq!(
            action("firefox"),
            Action::Script {
                run: r#"open -b org.mozilla.firefox 'https://example.com/a b?x='\''1'\'''"#.into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
        assert_eq!(
            action("desk"),
            Action::Script {
                run: "open ~/'Desktop/my shots'".into(),
                cwd: None,
                env: BTreeMap::new()
            }
        );
        assert_eq!(
            action("browse"),
            Action::Terminal {
                run: "cd ~/workspace".into(),
                cwd: None
            }
        );
    }

    #[test]
    fn ids_are_file_names_unique_regardless_of_case() {
        // Arrange
        let mut taken = HashSet::new();

        // Act
        let ids: Vec<String> = ["Today", "today", "a/b: c", "  ", ".hidden", "四柳 slack", "x||y"]
            .iter()
            .map(|base| unique_id(base, &mut taken))
            .collect();

        // Assert
        assert_eq!(
            ids,
            ["today", "today-2", "a-b-c", "alfred", "hidden", "四柳-slack", "x-y"]
        );
    }

    #[test]
    fn ids_follow_the_workflow_names_so_a_second_import_gives_the_same_ids() {
        // Arrange: 同じキーワードを持つワークフローを、名前と逆の順に渡す。
        let first = || {
            workflow(
                "B",
                vec![
                    keyword("k", "same", "B"),
                    object("a", "action.openurl", json!({ "url": "https://b.example" })),
                ],
                &[("k", "a")],
            )
        };
        let second = || {
            workflow(
                "A",
                vec![
                    keyword("k", "same", "A"),
                    object("a", "action.openurl", json!({ "url": "https://a.example" })),
                ],
                &[("k", "a")],
            )
        };

        // Act
        let one = convert(&[first(), second()]);
        let two = convert(&[second(), first()]);

        // Assert
        let pairs = |c: &Conversion| -> Vec<(String, String)> {
            c.commands
                .iter()
                .map(|i| (i.id.clone(), i.command.title.clone()))
                .collect()
        };
        assert_eq!(
            pairs(&one),
            [("same".into(), "A".into()), ("same-2".into(), "B".into())]
        );
        assert_eq!(pairs(&one), pairs(&two));
    }

    #[test]
    fn disabled_workflow_and_modifier_variants_are_reported() {
        // Arrange
        let mut disabled = workflow("Off", vec![], &[]);
        disabled.disabled = true;
        let info = json!({
            "name": "On",
            "objects": [
                keyword("k", "site", "Site"),
                object("a", "action.openurl", json!({ "url": "https://example.com" })),
                object("b", "action.openurl", json!({ "url": "https://example.com/alt" })),
            ],
            "connections": { "k": [
                { "destinationuid": "a", "modifiers": 0 },
                { "destinationuid": "b", "modifiers": 1_048_576 },
            ] },
        });
        let enabled = Workflow::from_json(&info).unwrap();

        // Act
        let conversion = convert(&[disabled, enabled]);

        // Assert
        assert_eq!(reasons(&conversion), ["Off — the workflow is disabled"]);
        assert_eq!(
            conversion.commands[0].warnings,
            ["the variants run with a modifier key held were not imported"]
        );
    }

    #[test]
    fn every_converted_command_is_a_valid_command_file() {
        // Arrange: 変換結果はそのまま保存される。読み戻せないものを作らない。
        let workflows = [workflow(
            "on",
            vec![
                keyword("k", "no||yes", "no"),
                object("a", "action.openurl", json!({ "url": "https://example.com" })),
                hotkey("h", CTRL_ALT_CMD, 49, "Space"),
                object("b", "action.launchfiles", json!({ "paths": ["~/Apps/My Tool.app"] })),
            ],
            &[("k", "a"), ("h", "b")],
        )];

        // Act
        let conversion = convert(&workflows);

        // Assert
        assert_eq!(conversion.commands.len(), 2);
        for imported in &conversion.commands {
            let text = render(&imported.command);
            assert_eq!(parse_command(&text), Ok(imported.command.clone()), "{text}");
        }
    }
}
