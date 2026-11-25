//! Spike: can yaml-edit rewrite a hand-written, commented command file without breaking
//! what the person wrote? Each case edits `BASE`, prints the result, and checks both the text
//! (comments and untouched lines) and the meaning (re-read with serde-saphyr, the parser the
//! app uses).

use serde_json::{json, Value};
use std::str::FromStr;
use yaml_edit::path::YamlPath;
use yaml_edit::Document;

const BASE: &str = r#"# Deploy the blog. Written by hand.
title: Deploy blog   # shown in the list
keyword: deploy

type: script
run: fab deploy
cwd: ~/workspace/blog
env:
  # token for gh
  GITHUB_TOKEN: op://Private/GitHub/token
  FOO: "bar"   # quoted on purpose
tags:
  - blog
  - deploy   # last tag
"#;

struct Outcome {
    name: &'static str,
    ok: bool,
    notes: Vec<String>,
    output: String,
}

fn parse(text: &str) -> Result<Value, String> {
    serde_saphyr::from_str::<Value>(text).map_err(|e| e.to_string())
}

/// Lines of `before` that are missing from `after`, excluding the lines the case is expected to touch.
fn lost_lines(before: &str, after: &str, expected_touched: &[&str]) -> Vec<String> {
    before
        .lines()
        .filter(|l| !expected_touched.iter().any(|t| l.contains(t)))
        .filter(|l| !after.lines().any(|a| a == *l))
        .map(|l| l.to_string())
        .collect()
}

fn case(
    name: &'static str,
    input: &str,
    expected_touched: &[&str],
    edit: impl FnOnce(&Document),
    check: impl FnOnce(&Value, &str) -> Vec<String>,
) -> Outcome {
    let mut notes = Vec::new();
    let doc = match Document::from_str(input) {
        Ok(d) => d,
        Err(e) => {
            return Outcome {
                name,
                ok: false,
                notes: vec![format!("parse failed: {e}")],
                output: String::new(),
            }
        }
    };
    edit(&doc);
    let output = doc.to_string();
    for l in lost_lines(input, &output, expected_touched) {
        notes.push(format!("untouched line changed or lost: {l:?}"));
    }
    match parse(&output) {
        Ok(v) => notes.extend(check(&v, &output)),
        Err(e) => notes.push(format!("output does not re-parse: {e}")),
    }
    Outcome {
        name,
        ok: notes.is_empty(),
        notes,
        output,
    }
}

fn expect_eq(v: &Value, pointer: &str, want: Value) -> Vec<String> {
    match v.pointer(pointer) {
        Some(got) if *got == want => vec![],
        got => vec![format!("{pointer}: want {want}, got {got:?}")],
    }
}

fn expect_text(output: &str, needle: &str) -> Vec<String> {
    if output.contains(needle) {
        vec![]
    } else {
        vec![format!("text missing: {needle:?}")]
    }
}

fn main() {
    let mut results = Vec::new();

    results.push(case(
        "01 no-op round trip is byte identical",
        BASE,
        &[],
        |_| {},
        |_, out| {
            if out == BASE {
                vec![]
            } else {
                vec!["output differs from input".into()]
            }
        },
    ));

    results.push(case(
        "02 change scalar that has an inline comment",
        BASE,
        &["title:"],
        |d| {
            d.as_mapping().unwrap().set("title", "Deploy the blog");
        },
        |v, out| {
            let mut n = expect_eq(v, "/title", json!("Deploy the blog"));
            n.extend(expect_text(out, "title: Deploy the blog   # shown in the list"));
            n
        },
    ));

    results.push(case(
        "03 change nested quoted scalar",
        BASE,
        &["FOO:"],
        |d| {
            d.as_mapping().unwrap().get_mapping("env").unwrap().set("FOO", "baz");
        },
        |v, out| {
            let mut n = expect_eq(v, "/env/FOO", json!("baz"));
            n.extend(expect_text(out, "# quoted on purpose"));
            n
        },
    ));

    results.push(case(
        "04 add top-level key (bool)",
        BASE,
        &[],
        |d| {
            d.as_mapping().unwrap().set("pin", true);
        },
        |v, _| expect_eq(v, "/pin", json!(true)),
    ));

    results.push(case(
        "05 add nested key",
        BASE,
        &[],
        |d| {
            d.as_mapping().unwrap().get_mapping("env").unwrap().set("NEW_VAR", "1");
        },
        |v, _| {
            let mut n = expect_eq(v, "/env/NEW_VAR", json!("1"));
            n.extend(expect_eq(v, "/tags/1", json!("deploy")));
            n
        },
    ));

    results.push(case(
        "06 remove a plain key",
        BASE,
        &["cwd:"],
        |d| {
            d.as_mapping().unwrap().remove("cwd");
        },
        |v, _| {
            if v.get("cwd").is_none() {
                vec![]
            } else {
                vec!["cwd still present".into()]
            }
        },
    ));

    results.push(case(
        "07 remove a key that has a comment line above it",
        BASE,
        &["GITHUB_TOKEN:", "# token for gh"],
        |d| {
            d.as_mapping()
                .unwrap()
                .get_mapping("env")
                .unwrap()
                .remove("GITHUB_TOKEN");
        },
        |v, out| {
            let mut n = vec![];
            if v.pointer("/env/GITHUB_TOKEN").is_some() {
                n.push("key still present".into());
            }
            n.extend(expect_eq(v, "/env/FOO", json!("bar")));
            if out.contains("# token for gh") {
                n.push("INFO: the comment above the removed key stays behind (orphan)".into());
            }
            n
        },
    ));

    results.push(case(
        "08 remove the last key of a nested mapping",
        BASE,
        &["FOO:"],
        |d| {
            d.as_mapping().unwrap().get_mapping("env").unwrap().remove("FOO");
        },
        |v, _| {
            let mut n = expect_eq(v, "/env/GITHUB_TOKEN", json!("op://Private/GitHub/token"));
            n.extend(expect_eq(v, "/tags/0", json!("blog")));
            n
        },
    ));

    results.push(case(
        "09 sequence push",
        BASE,
        &[],
        |d| {
            d.as_mapping().unwrap().get_sequence("tags").unwrap().push("ci");
        },
        |v, out| {
            let mut n = expect_eq(v, "/tags", json!(["blog", "deploy", "ci"]));
            n.extend(expect_text(out, "- deploy   # last tag"));
            n
        },
    ));

    results.push(case(
        "10 sequence remove first",
        BASE,
        &["- blog"],
        |d| {
            d.as_mapping().unwrap().get_sequence("tags").unwrap().remove(0);
        },
        |v, _| expect_eq(v, "/tags", json!(["deploy"])),
    ));

    // Strings that turn into another type or break the file when written out carelessly.
    let tricky: &[(&str, &str)] = &[
        ("yes-no", "no"),
        ("on-off", "on"),
        ("digits", "0123"),
        ("float", "1.0"),
        ("null", "null"),
        ("tilde", "~"),
        ("colon", "a: b"),
        ("hash", "x # not a comment"),
        ("leading-hash", "# heading"),
        ("japanese", "ブログをデプロイ"),
        ("empty", ""),
        ("quote", "say \"hi\" and 'bye'"),
        ("brace", "{not: flow}"),
        ("star", "*alias"),
        ("leading-space", " padded "),
        ("backslash-win-path", "C:\\Users\\me\\app.exe"),
        ("multiline", "echo one\necho two\n"),
        ("tab", "a\tb"),
    ];
    for (label, s) in tricky {
        let name: &'static str = Box::leak(format!("11 string keeps its type: {label}").into_boxed_str());
        let s = s.to_string();
        let want = s.clone();
        results.push(case(
            name,
            BASE,
            &["run:"],
            move |d| {
                d.as_mapping().unwrap().set("run", s.as_str());
            },
            move |v, _| expect_eq(v, "/run", json!(want)),
        ));
    }

    results.push(case(
        "12 set to the same value produces no diff",
        BASE,
        &[],
        |d| {
            let m = d.as_mapping().unwrap();
            m.set("keyword", "deploy");
            m.get_mapping("env").unwrap().set("FOO", "bar");
        },
        |_, out| {
            if out == BASE {
                vec![]
            } else {
                vec!["file changed although values are the same".into()]
            }
        },
    ));

    let block = "# c\nrun: |\n  echo one\n  echo two\ncwd: ~/x   # keep\n";
    results.push(case(
        "13 replace a block scalar",
        block,
        &["run:", "echo one", "echo two"],
        |d| {
            d.as_mapping().unwrap().set("run", "echo three\necho four\n");
        },
        |v, out| {
            let mut n = expect_eq(v, "/run", json!("echo three\necho four\n"));
            n.extend(expect_eq(v, "/cwd", json!("~/x")));
            n.extend(expect_text(out, "cwd: ~/x   # keep"));
            n
        },
    ));

    let crlf = BASE.replace('\n', "\r\n");
    let crlf_static: &'static str = Box::leak(crlf.into_boxed_str());
    results.push(case(
        "14 CRLF file: edit keeps CRLF everywhere",
        crlf_static,
        &["title:"],
        |d| {
            d.as_mapping().unwrap().set("title", "X");
            d.as_mapping().unwrap().set("pin", true);
        },
        |v, out| {
            let mut n = expect_eq(v, "/title", json!("X"));
            let bare_lf = out.replace("\r\n", "").contains('\n');
            if bare_lf {
                n.push("mixed line endings after edit".into());
            }
            n
        },
    ));

    results.push(case(
        "15 create nested mapping through a path",
        "title: A\n",
        &[],
        |d| {
            d.try_set_path("env.TOKEN", "op://v/i/f").unwrap();
        },
        |v, _| expect_eq(v, "/env/TOKEN", json!("op://v/i/f")),
    ));

    results.push(case(
        "16 insert a new key at a fixed position (after keyword)",
        BASE,
        &[],
        |d| {
            d.insert_after("keyword", "icon", "rocket");
        },
        |v, out| {
            let mut n = expect_eq(v, "/icon", json!("rocket"));
            let k = out.find("keyword:").unwrap_or(usize::MAX);
            let i = out.find("icon:").unwrap_or(0);
            let t = out.find("type:").unwrap_or(0);
            if !(k < i && i < t) {
                n.push("icon is not between keyword and type".into());
            }
            n
        },
    ));

    results.push(case(
        "17 rename key keeps its comment",
        BASE,
        &["title:"],
        |d| {
            d.as_mapping().unwrap().rename_key("title", "name");
        },
        |v, out| {
            let mut n = expect_eq(v, "/name", json!("Deploy blog"));
            n.extend(expect_text(out, "# shown in the list"));
            n
        },
    ));

    let flow = "title: A\nargs: [one, two]   # flow\nenv: {A: \"1\", B: \"2\"}\n";
    results.push(case(
        "18 flow collections: edit inside them",
        flow,
        &["args:", "env:"],
        |d| {
            let m = d.as_mapping().unwrap();
            m.get_sequence("args").unwrap().push("three");
            m.get_mapping("env").unwrap().set("B", "3");
        },
        |v, _| {
            let mut n = expect_eq(v, "/args", json!(["one", "two", "three"]));
            n.extend(expect_eq(v, "/env", json!({"A": "1", "B": "3"})));
            n
        },
    ));

    let no_trailing_newline = "title: A\nkeyword: a";
    results.push(case(
        "19 file without trailing newline: add key",
        no_trailing_newline,
        &["keyword:"],
        |d| {
            d.as_mapping().unwrap().set("pin", true);
        },
        |v, _| {
            let mut n = expect_eq(v, "/keyword", json!("a"));
            n.extend(expect_eq(v, "/pin", json!(true)));
            n
        },
    ));

    results.push(case(
        "20 many edits in one pass (what a settings form would do)",
        BASE,
        &["title:", "cwd:", "FOO:", "run:"],
        |d| {
            let m = d.as_mapping().unwrap();
            m.set("title", "Deploy blog (prod)");
            m.set("run", "fab deploy --prod");
            m.remove("cwd");
            m.set("pin", true);
            let env = m.get_mapping("env").unwrap();
            env.set("FOO", "qux");
            env.set("STAGE", "prod");
            m.get_sequence("tags").unwrap().push("prod");
        },
        |v, out| {
            let want = json!({
                "title": "Deploy blog (prod)",
                "keyword": "deploy",
                "type": "script",
                "run": "fab deploy --prod",
                "env": {"GITHUB_TOKEN": "op://Private/GitHub/token", "FOO": "qux", "STAGE": "prod"},
                "tags": ["blog", "deploy", "prod"],
                "pin": true
            });
            let mut n = if *v == want {
                vec![]
            } else {
                vec![format!("meaning differs: {v}")]
            };
            for c in [
                "# Deploy the blog. Written by hand.",
                "# shown in the list",
                "# token for gh",
                "# quoted on purpose",
                "# last tag",
            ] {
                n.extend(expect_text(out, c));
            }
            n
        },
    ));

    let verbose = std::env::args().any(|a| a == "-v");
    let mut failed = 0;
    for r in &results {
        let info_only = !r.notes.is_empty() && r.notes.iter().all(|n| n.starts_with("INFO"));
        let status = if r.ok {
            "PASS"
        } else if info_only {
            "INFO"
        } else {
            failed += 1;
            "FAIL"
        };
        println!("[{status}] {}", r.name);
        for n in &r.notes {
            println!("        {n}");
        }
        if verbose || status != "PASS" {
            println!("        ---- output ----");
            for l in r.output.lines() {
                println!("        | {l}");
            }
        }
    }
    println!("\n{} cases, {} failed", results.len(), failed);
}
