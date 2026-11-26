use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct CommentRatchet {
    max_comments_by_file: BTreeMap<String, usize>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CommentOccurrence {
    start_line: usize,
    end_line: usize,
    line_count: usize,
    preview: String,
}

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct RatchetReport {
    violations: Vec<String>,
    tightenable: Vec<String>,
    tightened: BTreeMap<String, usize>,
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(Path::parent)
        .expect("larik-core must live under <workspace>/crates/larik-core")
        .to_path_buf()
}

fn collect_rust_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    let mut children: Vec<PathBuf> = entries
        .filter_map(|entry| entry.ok().map(|e| e.path()))
        .collect();
    children.sort();
    for path in children {
        if path.is_dir() {
            if path.file_name().and_then(|n| n.to_str()) == Some("target") {
                continue;
            }
            collect_rust_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

fn is_ident_continue(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn scan_rust_comments(source: &str) -> Vec<CommentOccurrence> {
    let bytes = source.as_bytes();
    let n = bytes.len();
    let mut i = 0usize;
    let mut line = 1usize;
    let mut results = Vec::new();

    while i < n {
        let b = bytes[i];

        if b == b'\n' {
            line += 1;
            i += 1;
            continue;
        }

        let prev_is_ident = i > 0 && is_ident_continue(bytes[i - 1]);

        let raw_prefix = if !prev_is_ident && b == b'r' {
            Some(1usize)
        } else if !prev_is_ident && (b == b'b' || b == b'c') && i + 1 < n && bytes[i + 1] == b'r' {
            Some(2usize)
        } else {
            None
        };

        if let Some(plen) = raw_prefix {
            let mut j = i + plen;
            let mut hashes = 0usize;
            while j < n && bytes[j] == b'#' {
                hashes += 1;
                j += 1;
            }
            if j < n && bytes[j] == b'"' {
                i = j + 1;
                while i < n {
                    if bytes[i] == b'\n' {
                        line += 1;
                        i += 1;
                    } else if bytes[i] == b'"' {
                        let mut h = 0usize;
                        while h < hashes && i + 1 + h < n && bytes[i + 1 + h] == b'#' {
                            h += 1;
                        }
                        if h == hashes {
                            i += 1 + hashes;
                            break;
                        }
                        i += 1;
                    } else {
                        i += 1;
                    }
                }
                continue;
            }
        }

        if !prev_is_ident && (b == b'b' || b == b'c') && i + 1 < n && bytes[i + 1] == b'"' {
            i += 2;
            while i < n {
                if bytes[i] == b'\\' {
                    if i + 1 < n && bytes[i + 1] == b'\n' {
                        line += 1;
                    }
                    i = (i + 2).min(n);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        if b == b'"' {
            i += 1;
            while i < n {
                if bytes[i] == b'\\' {
                    if i + 1 < n && bytes[i + 1] == b'\n' {
                        line += 1;
                    }
                    i = (i + 2).min(n);
                } else if bytes[i] == b'"' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        if !prev_is_ident && b == b'b' && i + 1 < n && bytes[i + 1] == b'\'' {
            i += 2;
            while i < n {
                if bytes[i] == b'\\' {
                    i = (i + 2).min(n);
                } else if bytes[i] == b'\'' {
                    i += 1;
                    break;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            continue;
        }

        if b == b'\'' {
            if i + 1 < n && bytes[i + 1] == b'\\' {
                i += 2;
                while i < n && bytes[i] != b'\'' && bytes[i] != b'\n' {
                    i += 1;
                }
                if i < n && bytes[i] == b'\'' {
                    i += 1;
                }
                continue;
            }
            if let Some(rest) = source.get(i + 1..) {
                let mut chars = rest.chars();
                if let Some(ch) = chars.next() {
                    if ch != '\n' && ch != '\'' && chars.next() == Some('\'') {
                        i += 1 + ch.len_utf8() + 1;
                        continue;
                    }
                }
            }
            i += 1;
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'/' {
            let start = i;
            let start_line = line;
            while i < n && bytes[i] != b'\n' {
                i += 1;
            }
            let preview = source[start..i].trim_end_matches('\r').trim().to_string();
            results.push(CommentOccurrence {
                start_line,
                end_line: start_line,
                line_count: 1,
                preview,
            });
            continue;
        }

        if b == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
            let start = i;
            let start_line = line;
            let mut depth = 1usize;
            i += 2;
            while i < n && depth > 0 {
                if bytes[i] == b'/' && i + 1 < n && bytes[i + 1] == b'*' {
                    depth += 1;
                    i += 2;
                } else if bytes[i] == b'*' && i + 1 < n && bytes[i + 1] == b'/' {
                    depth -= 1;
                    i += 2;
                } else {
                    if bytes[i] == b'\n' {
                        line += 1;
                    }
                    i += 1;
                }
            }
            let end_line = line;
            let first_line = source[start..i]
                .lines()
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            results.push(CommentOccurrence {
                start_line,
                end_line,
                line_count: end_line - start_line + 1,
                preview: first_line,
            });
            continue;
        }

        i += 1;
    }

    results
}

fn evaluate_ratchet(
    actual_by_file: &BTreeMap<String, Vec<CommentOccurrence>>,
    ratchet: &CommentRatchet,
) -> RatchetReport {
    let mut report = RatchetReport::default();

    for (rel_path, comments) in actual_by_file {
        let actual_lines: usize = comments.iter().map(|c| c.line_count).sum();
        let allowed = ratchet
            .max_comments_by_file
            .get(rel_path)
            .copied()
            .unwrap_or(0);

        if actual_lines > allowed {
            let sample = comments
                .iter()
                .take(3)
                .map(|c| format!("L{}: {}", c.start_line, c.preview))
                .collect::<Vec<_>>()
                .join("; ");
            report.violations.push(format!(
                "{rel_path}: found {actual_lines} comment line(s), ratchet allows {allowed} ({sample})"
            ));
        } else if actual_lines < allowed {
            report.tightenable.push(format!(
                "{rel_path}: reduced from {allowed} to {actual_lines} comment line(s)"
            ));
        }

        if actual_lines > 0 && actual_lines <= allowed {
            report.tightened.insert(rel_path.clone(), actual_lines);
        }
    }

    for (rel_path, &allowed) in &ratchet.max_comments_by_file {
        if !actual_by_file.contains_key(rel_path) {
            report.tightenable.push(format!(
                "{rel_path}: listed with ceiling {allowed} but file no longer exists"
            ));
        } else if allowed == 0 {
            report.tightenable.push(format!(
                "{rel_path}: explicit zero ceiling should be removed from ratchet"
            ));
        }
    }

    report
}

#[test]
fn enforce_workspace_comment_ratchet() {
    let root = workspace_root();
    let ratchet_path = root.join(".comment-ratchet.json");
    let ratchet_raw = fs::read_to_string(&ratchet_path)
        .unwrap_or_else(|e| panic!("failed to read {}: {e}", ratchet_path.display()));
    let ratchet: CommentRatchet = serde_json::from_str(&ratchet_raw)
        .unwrap_or_else(|e| panic!("failed to parse {}: {e}", ratchet_path.display()));

    let mut rs_files = Vec::new();
    collect_rust_files(&root.join("crates"), &mut rs_files);
    assert!(
        !rs_files.is_empty(),
        "expected Rust source files under crates/"
    );

    let mut actual_by_file = BTreeMap::new();
    for path in rs_files {
        let rel = path
            .strip_prefix(&root)
            .expect("path must be inside workspace root")
            .to_string_lossy()
            .replace('\\', "/");
        let source = fs::read_to_string(&path)
            .unwrap_or_else(|e| panic!("failed to read {}: {e}", path.display()));
        actual_by_file.insert(rel, scan_rust_comments(&source));
    }

    let report = evaluate_ratchet(&actual_by_file, &ratchet);

    if !report.violations.is_empty() {
        panic!(
            "Code comments are banned by the comment ratchet lint.\n\
             Delete tautology/stale comments, file an issue for excuses, or record non-obvious decisions in docs/adr/.\n\
             Violations:\n  - {}",
            report.violations.join("\n  - ")
        );
    }

    if !report.tightenable.is_empty() {
        if std::env::var("UPDATE_COMMENT_RATCHET").as_deref() == Ok("1") {
            let updated = CommentRatchet {
                max_comments_by_file: report.tightened,
            };
            let formatted = format!(
                "{}\n",
                serde_json::to_string_pretty(&updated).expect("serialize ratchet")
            );
            fs::write(&ratchet_path, formatted)
                .unwrap_or_else(|e| panic!("failed to write {}: {e}", ratchet_path.display()));
        } else {
            panic!(
                "Comment ratchet has stale headroom and must be tightened downward.\n\
                 Run `UPDATE_COMMENT_RATCHET=1 cargo test -p larik-core --test comment_lint` to update .comment-ratchet.json:\n  - {}",
                report.tightenable.join("\n  - ")
            );
        }
    }
}

#[test]
fn scanner_ignores_strings_raw_strings_chars_and_lifetimes() {
    let code = concat!(
        "fn demo<'a>(s: &'a str) -> &'static str {\n",
        "    let url = \"https://example.com /* not a comment */\";\n",
        "    let raw = r#\"// also not a comment /* nope */\"#;\n",
        "    let bytes = b\"// byte string\";\n",
        "    let slash = '/';\n",
        "    let esc = '\\'';\n",
        "    let _ = (s, url, raw, bytes, slash, esc);\n",
        "    \"ok\"\n",
        "}\n",
    );
    assert!(scan_rust_comments(code).is_empty());
}

#[test]
fn scanner_detects_line_doc_inline_and_nested_block_comments() {
    let code = concat!(
        "/// doc comment\n",
        "//! inner doc\n",
        "fn f() {\n",
        "    let x = 1; // inline\n",
        "    /* outer /* nested */ still comment\n",
        "       second line */\n",
        "    let _ = x;\n",
        "}\n",
    );
    let found = scan_rust_comments(code);
    assert_eq!(found.len(), 4);
    assert_eq!(found[0].start_line, 1);
    assert_eq!(found[1].start_line, 2);
    assert_eq!(found[2].start_line, 4);
    assert_eq!(found[3].start_line, 5);
    assert_eq!(found[3].end_line, 6);
    assert_eq!(found[3].line_count, 2);
}

#[test]
fn ratchet_blocks_increases_allows_baseline_and_requires_tightening() {
    let mut ratchet = CommentRatchet::default();
    ratchet
        .max_comments_by_file
        .insert("crates/a/src/lib.rs".into(), 2);
    ratchet
        .max_comments_by_file
        .insert("crates/b/src/lib.rs".into(), 3);
    ratchet
        .max_comments_by_file
        .insert("crates/deleted/src/lib.rs".into(), 1);

    let mut actual = BTreeMap::new();
    actual.insert(
        "crates/a/src/lib.rs".into(),
        scan_rust_comments("// one\n// two\n"),
    );
    actual.insert(
        "crates/b/src/lib.rs".into(),
        scan_rust_comments("// only one now\n"),
    );
    actual.insert(
        "crates/c/src/lib.rs".into(),
        scan_rust_comments("// newly added comment\n"),
    );

    let report = evaluate_ratchet(&actual, &ratchet);
    assert_eq!(report.violations.len(), 1);
    assert!(report.violations[0].contains("crates/c/src/lib.rs"));
    assert_eq!(report.tightenable.len(), 2);
    assert_eq!(report.tightened.get("crates/a/src/lib.rs"), Some(&2));
    assert_eq!(report.tightened.get("crates/b/src/lib.rs"), Some(&1));
    assert!(!report.tightened.contains_key("crates/deleted/src/lib.rs"));
}
