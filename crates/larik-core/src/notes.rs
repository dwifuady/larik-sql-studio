//! Statement-anchored and tab-level notes model + anchoring/rebasing engine.
//!
//! Ported from WinUI `Larik.Core/NoteRecords.cs`. All offsets are UTF-8 byte indices
//! matching `gpui` / `EditorState` and `larik_data::TextSpan`, with strict char-boundary
//! clamping so multi-byte characters can never cause a slice panic.

use chrono::{DateTime, Duration, Local, Utc};
use larik_data::EditorLanguage;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

pub const MAX_QUOTE_CHARS: usize = 4_000;
pub const CONTEXT_CHARS: usize = 32;
pub const MAX_SEARCH_HITS: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteKind {
    Note,
    Todo,
    Decision,
    Observation,
}

impl Default for NoteKind {
    fn default() -> Self {
        Self::Note
    }
}

impl NoteKind {
    pub const ALL: [Self; 4] = [Self::Note, Self::Todo, Self::Decision, Self::Observation];

    pub fn as_str(self) -> &'static str {
        match self {
            Self::Note => "note",
            Self::Todo => "todo",
            Self::Decision => "decision",
            Self::Observation => "observation",
        }
    }

    pub fn parse(raw: &str) -> Self {
        match raw.trim().to_ascii_lowercase().as_str() {
            "todo" => Self::Todo,
            "decision" => Self::Decision,
            "observation" => Self::Observation,
            _ => Self::Note,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::Note => "Note",
            Self::Todo => "TODO",
            Self::Decision => "Decision",
            Self::Observation => "Observation",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NoteAnchorState {
    Anchored,
    Moved,
    Lost,
    Tab,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteAnchor {
    pub start: usize,
    pub length: usize,
    pub quote: String,
    pub prefix: String,
    pub suffix: String,
    pub line_hint: usize,
    #[serde(default)]
    pub is_tab: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ResolvedNoteAnchor {
    pub start: usize,
    pub length: usize,
    pub state: NoteAnchorState,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NoteRecord {
    pub id: String,
    pub space_id: String,
    pub tab_id: String,
    pub kind: NoteKind,
    pub body: String,
    pub anchor: NoteAnchor,
    pub is_unanchored: bool,
    pub created_at: String,
    pub updated_at: String,
}

impl NoteRecord {
    pub fn new(space_id: impl Into<String>, tab_id: impl Into<String>, anchor: NoteAnchor) -> Self {
        let now = crate::now_rfc3339();
        let is_unanchored = !anchor.is_tab && (anchor.length == 0 || anchor.quote.is_empty());
        Self {
            id: Uuid::new_v4().simple().to_string(),
            space_id: space_id.into(),
            tab_id: tab_id.into(),
            kind: NoteKind::Note,
            body: String::new(),
            anchor,
            is_unanchored,
            created_at: now.clone(),
            updated_at: now,
        }
    }
}

/// Formats a badge count capped at `"99+"` (matching WinUI `TabViewModel.NotesBadgeText`).
pub fn format_badge_count(count: usize) -> String {
    if count > 99 {
        "99+".to_string()
    } else {
        count.to_string()
    }
}

/// Clamps `byte_offset` down to the nearest valid UTF-8 character boundary within `text`.
pub fn floor_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut idx = byte_offset.min(text.len());
    while idx > 0 && !text.is_char_boundary(idx) {
        idx -= 1;
    }
    idx
}

/// Clamps `byte_offset` up to the nearest valid UTF-8 character boundary within `text`.
pub fn ceil_char_boundary(text: &str, byte_offset: usize) -> usize {
    let mut idx = byte_offset.min(text.len());
    while idx < text.len() && !text.is_char_boundary(idx) {
        idx += 1;
    }
    idx
}

/// Creates a tab-level scratchpad anchor (`is_tab = true`).
pub fn tab_note_anchor() -> NoteAnchor {
    NoteAnchor {
        start: 0,
        length: 0,
        quote: String::new(),
        prefix: String::new(),
        suffix: String::new(),
        line_hint: 0,
        is_tab: true,
    }
}

/// Returns the 1-based line number of `byte_offset` in `text`, handling `\r\n`, `\n`, and lone `\r`.
pub fn line_of(text: &str, byte_offset: usize) -> usize {
    let limit = floor_char_boundary(text, byte_offset);
    let bytes = &text.as_bytes()[..limit];
    let mut line = 1usize;
    let mut i = 0usize;
    while i < bytes.len() {
        match bytes[i] {
            b'\r' => {
                line += 1;
                if i + 1 < bytes.len() && bytes[i + 1] == b'\n' {
                    i += 1;
                }
            }
            b'\n' => {
                line += 1;
            }
            _ => {}
        }
        i += 1;
    }
    line
}

/// Returns `(start, length)` in UTF-8 bytes for the line containing `byte_offset`,
/// excluding trailing `\r`/`\n`.
pub fn line_span_at(text: &str, byte_offset: usize) -> (usize, usize) {
    if text.is_empty() {
        return (0, 0);
    }
    let pos = floor_char_boundary(text, byte_offset);
    let bytes = text.as_bytes();

    let mut start = pos;
    while start > 0 && bytes[start - 1] != b'\n' && bytes[start - 1] != b'\r' {
        start -= 1;
    }

    let mut end = pos;
    while end < bytes.len() && bytes[end] != b'\n' && bytes[end] != b'\r' {
        end += 1;
    }

    (start, end.saturating_sub(start))
}

fn take_prefix_chars(slice: &str, max_chars: usize) -> String {
    let mut chars: Vec<char> = slice.chars().rev().take(max_chars).collect();
    chars.reverse();
    chars.into_iter().collect()
}

fn take_first_chars(slice: &str, max_chars: usize) -> String {
    slice.chars().take(max_chars).collect()
}

/// Captures a fingerprint anchor (`quote`, `prefix`, `suffix`, `line_hint`) for `[start, start+length)`.
pub fn capture_anchor(text: &str, start: usize, length: usize) -> NoteAnchor {
    let safe_start = floor_char_boundary(text, start);
    let safe_end = ceil_char_boundary(text, safe_start.saturating_add(length).min(text.len()));
    let safe_len = safe_end.saturating_sub(safe_start);

    let quote = take_first_chars(&text[safe_start..safe_end], MAX_QUOTE_CHARS);
    let prefix = take_prefix_chars(&text[..safe_start], CONTEXT_CHARS);
    let suffix = take_first_chars(&text[safe_end..], CONTEXT_CHARS);
    let line_hint = line_of(text, safe_start);

    NoteAnchor {
        start: safe_start,
        length: safe_len,
        quote,
        prefix,
        suffix,
        line_hint,
        is_tab: false,
    }
}

/// Rebases `anchor` across a text edit from `old_text` to `new_text` using a single-splice diff.
///
/// Boundary rules (matching WinUI `NoteAnchoring.Rebase`):
/// - Pure insertion at `anchor.start` shifts the anchor forward.
/// - Pure insertion at `anchor.start + anchor.length` leaves the anchor untouched.
/// - Pure insertion strictly inside expands the anchor.
/// - If a deletion/replacement collapses the anchor to empty, attempts `resolve_anchor` rescue
///   (e.g., whole-document reformat) before collapsing to zero-length (`Lost`).
///
/// Returns `(rebased_anchor, changed)`.
pub fn rebase_anchor(anchor: &NoteAnchor, old_text: &str, new_text: &str) -> (NoteAnchor, bool) {
    if anchor.is_tab || old_text == new_text {
        return (anchor.clone(), false);
    }

    // Compute common prefix & non-overlapping common suffix at char boundaries.
    let prefix = common_prefix_bytes(old_text, new_text);
    let suffix = common_suffix_bytes(&old_text[prefix..], &new_text[prefix..]);

    let old_middle = old_text.len() - prefix - suffix;
    let new_middle = new_text.len() - prefix - suffix;
    let delta: isize = new_middle as isize - old_middle as isize;
    let splice_end = prefix + old_middle;

    let start = anchor.start.min(old_text.len());
    let end = start.saturating_add(anchor.length).min(old_text.len());

    let (raw_start, raw_end) = if old_middle == 0 {
        // Pure insertion at `prefix`
        if prefix <= start {
            (start + new_middle, end + new_middle)
        } else if prefix >= end {
            (start, end)
        } else {
            // Strictly inside
            (start, end + new_middle)
        }
    } else if prefix <= start && end <= splice_end {
        // The entire anchor falls inside the replaced region `[prefix, splice_end]`.
        // First try rescuing the original quote (handles whole-doc format / paste / moves).
        let rescued = resolve_anchor(anchor, new_text);
        if rescued.state != NoteAnchorState::Lost && rescued.length > 0 {
            let updated = capture_anchor(new_text, rescued.start, rescued.length);
            let changed = &updated != anchor;
            return (updated, changed);
        }
        // If the user replaced the exact anchored span with new text, track the replacement;
        // if it was deleted or wiped out by a broader rewrite, collapse to Lost.
        if prefix == start && end == splice_end && new_middle > 0 {
            (prefix, prefix + new_middle)
        } else {
            (prefix, prefix)
        }
    } else {
        let ns = if start <= prefix {
            start
        } else if start < splice_end {
            prefix
        } else {
            (start as isize + delta).max(0) as usize
        };
        let ne = if end <= prefix {
            end
        } else if end < splice_end {
            prefix + new_middle
        } else {
            (end as isize + delta).max(0) as usize
        };
        (ns, ne)
    };

    let new_start = floor_char_boundary(new_text, raw_start.min(new_text.len()));
    let new_end = ceil_char_boundary(new_text, raw_end.max(new_start).min(new_text.len()));

    let updated = if new_end > new_start {
        capture_anchor(new_text, new_start, new_end - new_start)
    } else {
        // Rescue attempt before declaring lost (handles format / wholesale rewrite)
        let rescued = resolve_anchor(anchor, new_text);
        if rescued.state != NoteAnchorState::Lost && rescued.length > 0 {
            capture_anchor(new_text, rescued.start, rescued.length)
        } else {
            capture_anchor(new_text, new_start, 0)
        }
    };

    let changed = &updated != anchor;
    (updated, changed)
}

fn common_prefix_bytes(a: &str, b: &str) -> usize {
    let mut matched = 0usize;
    for (ca, cb) in a.chars().zip(b.chars()) {
        if ca != cb {
            break;
        }
        matched += ca.len_utf8();
    }
    matched
}

fn common_suffix_bytes(a: &str, b: &str) -> usize {
    let mut matched = 0usize;
    for (ca, cb) in a.chars().rev().zip(b.chars().rev()) {
        if ca != cb {
            break;
        }
        matched += ca.len_utf8();
    }
    matched
}

/// Resolves an anchor against `text`, returning `Anchored`, `Moved`, `Lost`, or `Tab`.
pub fn resolve_anchor(anchor: &NoteAnchor, text: &str) -> ResolvedNoteAnchor {
    if anchor.is_tab {
        return ResolvedNoteAnchor {
            start: 0,
            length: 0,
            state: NoteAnchorState::Tab,
        };
    }

    let clamped_start = floor_char_boundary(text, anchor.start.min(text.len()));
    if anchor.quote.is_empty() || anchor.length == 0 {
        return ResolvedNoteAnchor {
            start: clamped_start,
            length: 0,
            state: NoteAnchorState::Lost,
        };
    }

    // 1. Exact ordinal match at recorded offset.
    if anchor.start <= text.len() && text.is_char_boundary(anchor.start) {
        let rest = &text[anchor.start..];
        let expected_quote = if anchor.length < anchor.quote.len()
            && anchor.quote.is_char_boundary(anchor.length)
        {
            &anchor.quote[..anchor.length]
        } else {
            anchor.quote.as_str()
        };
        if rest.starts_with(expected_quote) {
            let resolved_len = anchor
                .length
                .max(expected_quote.len())
                .min(rest.len());
            let resolved_len = ceil_char_boundary(rest, resolved_len);
            return ResolvedNoteAnchor {
                start: anchor.start,
                length: resolved_len,
                state: NoteAnchorState::Anchored,
            };
        }
    }

    // 2. Overlapping scan for quote anywhere in `text` (up to MAX_SEARCH_HITS).
    let hits = find_all_overlapping(text, &anchor.quote, MAX_SEARCH_HITS);
    if hits.is_empty() {
        return ResolvedNoteAnchor {
            start: clamped_start,
            length: 0,
            state: NoteAnchorState::Lost,
        };
    }

    let best_start = if hits.len() == 1 {
        hits[0]
    } else {
        let mut best = hits[0];
        let mut best_score = -1isize;
        for &hit in &hits {
            let score = context_score(text, hit, anchor);
            if score > best_score {
                best_score = score;
                best = hit;
            }
        }
        best
    };

    let avail = text.len().saturating_sub(best_start);
    let desired = anchor.length.max(anchor.quote.len()).min(avail);
    let resolved_len = ceil_char_boundary(&text[best_start..], desired);

    ResolvedNoteAnchor {
        start: best_start,
        length: resolved_len,
        state: NoteAnchorState::Moved,
    }
}

fn find_all_overlapping(text: &str, needle: &str, limit: usize) -> Vec<usize> {
    if needle.is_empty() || needle.len() > text.len() || limit == 0 {
        return Vec::new();
    }
    let mut hits = Vec::new();
    let mut offset = 0usize;
    while offset <= text.len() && hits.len() < limit {
        let Some(rel) = text[offset..].find(needle) else {
            break;
        };
        let hit = offset + rel;
        hits.push(hit);
        // Advance by one UTF-8 char for overlapping match scan.
        let step = text[hit..].chars().next().map_or(1, char::len_utf8);
        offset = hit + step;
    }
    hits
}

fn context_score(text: &str, hit: usize, anchor: &NoteAnchor) -> isize {
    let mut score = 0isize;
    if !anchor.prefix.is_empty() && text[..hit].ends_with(&anchor.prefix) {
        score += anchor.prefix.chars().count() as isize + 1;
    }
    let after = hit.saturating_add(anchor.quote.len());
    if !anchor.suffix.is_empty() && after <= text.len() && text[after..].starts_with(&anchor.suffix)
    {
        score += anchor.suffix.chars().count() as isize + 1;
    }
    score
}

/// Collapses newlines, tabs, and whitespace runs to a single space for card/popover preview.
pub fn quote_preview(quote: &str) -> String {
    let mut out = String::with_capacity(quote.len().min(160));
    let mut in_ws = false;
    for ch in quote.chars() {
        if ch.is_whitespace() {
            if !out.is_empty() {
                in_ws = true;
            }
        } else {
            if in_ws {
                out.push(' ');
                in_ws = false;
            }
            out.push(ch);
        }
    }
    out
}

/// Computes the human-readable anchor description matching WinUI `StudioViewModel.DescribeAnchor`:
/// - Tab note -> `"This tab"`
/// - Lost note -> `"Anchor lost"`
/// - Anchored/Moved -> `"Statement {index} · Line {a}"` / `"Statement {index} · Lines {a}–{b}"`
///   (or just `"Line {a}"` / `"Lines {a}–{b}"` if no statement covers it).
pub fn describe_anchor(
    anchor: &NoteAnchor,
    state: NoteAnchorState,
    text: &str,
    language: &dyn EditorLanguage,
) -> String {
    if anchor.is_tab {
        return "This tab".to_string();
    }
    if state == NoteAnchorState::Lost || anchor.length == 0 {
        return "Anchor lost".to_string();
    }

    let safe_start = floor_char_boundary(text, anchor.start.min(text.len()));
    let safe_end = ceil_char_boundary(
        text,
        safe_start.saturating_add(anchor.length).min(text.len()),
    );
    let start_line = line_of(text, safe_start);
    let end_probe = if safe_end > safe_start {
        floor_char_boundary(text, safe_end - 1)
    } else {
        safe_start
    };
    let end_line = line_of(text, end_probe).max(start_line);

    let lines_label = if end_line > start_line {
        format!("Lines {start_line}–{end_line}")
    } else {
        format!("Line {start_line}")
    };

    let statements = language.split_statements(text);
    let stmt_index = statements
        .iter()
        .filter(|stmt| stmt.start <= safe_start)
        .count();

    if stmt_index > 0 {
        format!("Statement {stmt_index} · {lines_label}")
    } else {
        lines_label
    }
}

/// Relative timestamp label matching WinUI `NoteViewModel.EditedLabel`.
pub fn edited_label(updated_at: &str, now: DateTime<Utc>) -> String {
    let Ok(parsed) = DateTime::parse_from_rfc3339(updated_at) else {
        return "just now".to_string();
    };
    let utc = parsed.with_timezone(&Utc);
    let age = now.signed_duration_since(utc);
    if age < Duration::minutes(1) {
        "just now".to_string()
    } else if age < Duration::hours(1) {
        format!("{} min ago", age.num_minutes().max(1))
    } else if age < Duration::hours(24) {
        format!("{}h ago", age.num_hours().max(1))
    } else if age < Duration::hours(48) {
        "yesterday".to_string()
    } else if age < Duration::days(30) {
        format!("{}d ago", age.num_days().max(1))
    } else {
        utc.with_timezone(&Local)
            .format("%b %-d, %Y")
            .to_string()
    }
}

#[cfg(test)]
mod tests {
    use larik_data::sql_language::SqlLanguage;

    use super::*;

    #[test]
    fn capture_and_resolve_exact_and_moved_and_disambiguated() {
        let doc = "SELECT 1;\nSELECT id FROM dbo.orders WHERE status = 'open';\nSELECT id FROM dbo.orders WHERE status = 'open';\nSELECT 2;";
        let target = "SELECT id FROM dbo.orders WHERE status = 'open';";
        let first = doc.find(target).unwrap();
        let second = doc[first + 1..].find(target).unwrap() + first + 1;

        let anchor = capture_anchor(doc, second, target.len());
        assert_eq!(anchor.line_hint, 3);
        assert_eq!(
            resolve_anchor(&anchor, doc),
            ResolvedNoteAnchor {
                start: second,
                length: target.len(),
                state: NoteAnchorState::Anchored
            }
        );

        // Prepend lines so the anchor shifts; context suffix ("\nSELECT 2;") disambiguates the second copy.
        let shifted = format!("-- header\n{doc}");
        let res = resolve_anchor(&anchor, &shifted);
        assert_eq!(res.state, NoteAnchorState::Moved);
        assert_eq!(res.start, second + "-- header\n".len());
        assert_eq!(res.length, target.len());

        // Removing the text marks the anchor Lost.
        let lost = resolve_anchor(&anchor, "SELECT 99;");
        assert_eq!(lost.state, NoteAnchorState::Lost);
    }

    #[test]
    fn rebase_respects_boundary_rules_and_rescues_on_rewrite() {
        let old = "SELECT a FROM t;\nSELECT b FROM t;";
        let start = old.find("SELECT b").unwrap();
        let len = "SELECT b FROM t;".len();
        let anchor = capture_anchor(old, start, len);

        // 1. Insert at exact start -> shifts forward.
        let inserted_at_start = format!("{}  {}", &old[..start], &old[start..]);
        let (rebased, changed) = rebase_anchor(&anchor, old, &inserted_at_start);
        assert!(changed);
        assert_eq!(rebased.start, start + 2);
        assert_eq!(rebased.length, len);
        assert_eq!(rebased.quote, "SELECT b FROM t;");

        // 2. Insert at exact end -> untouched.
        let inserted_at_end = format!("{old}\nSELECT c;");
        let (rebased, changed) = rebase_anchor(&anchor, old, &inserted_at_end);
        assert_eq!(rebased.start, start);
        assert_eq!(rebased.length, len);
        assert_eq!(rebased.quote, "SELECT b FROM t;");
        assert!(changed); // suffix refreshed

        // 3. Insert strictly inside -> expands.
        let inside_pos = start + "SELECT b".len();
        let inserted_inside = format!("{}, b2{}", &old[..inside_pos], &old[inside_pos..]);
        let (rebased, changed) = rebase_anchor(&anchor, old, &inserted_inside);
        assert!(changed);
        assert_eq!(rebased.start, start);
        assert_eq!(rebased.length, len + 4);
        assert_eq!(rebased.quote, "SELECT b, b2 FROM t;");

        // 4. Whole-doc rewrite where statement still exists elsewhere -> rescued!
        let rewritten = "-- reformatted\nSELECT x FROM y;\n\nSELECT b FROM t;\n";
        let (rescued, _) = rebase_anchor(&anchor, old, rewritten);
        assert_eq!(rescued.quote, "SELECT b FROM t;");
        assert_eq!(rescued.start, rewritten.find("SELECT b FROM t;").unwrap());

        // 5. Deleting the anchored statement completely -> collapses to length 0 (Lost).
        let deleted = "SELECT a FROM t;\n";
        let (lost_anchor, _) = rebase_anchor(&anchor, old, deleted);
        assert_eq!(lost_anchor.length, 0);
        assert_eq!(
            resolve_anchor(&lost_anchor, deleted).state,
            NoteAnchorState::Lost
        );
    }

    #[test]
    fn non_ascii_utf8_never_panics() {
        let doc = "SELECT 'héllo 🚀' AS greeting;\nSELECT '日本語';";
        let start = doc.find("SELECT '日").unwrap();
        let anchor = capture_anchor(doc, start, doc.len() - start);
        assert_eq!(resolve_anchor(&anchor, doc).state, NoteAnchorState::Anchored);

        let edited = format!("-- café\n{doc}");
        let (rebased, _) = rebase_anchor(&anchor, doc, &edited);
        assert_eq!(rebased.quote, "SELECT '日本語';");
        assert_eq!(rebased.line_hint, 3);
    }

    #[test]
    fn describe_anchor_and_quote_preview_match_winui() {
        let lang = SqlLanguage::tsql();
        let doc = "SELECT 1;\n\nSELECT\n  id,\n  name\nFROM dbo.users;";
        let start = doc.find("SELECT\n").unwrap();
        let anchor = capture_anchor(doc, start, doc.len() - start);

        assert_eq!(
            describe_anchor(&anchor, NoteAnchorState::Anchored, doc, &lang),
            "Statement 2 · Lines 3–6"
        );
        assert_eq!(
            describe_anchor(&tab_note_anchor(), NoteAnchorState::Tab, doc, &lang),
            "This tab"
        );
        assert_eq!(
            describe_anchor(&anchor, NoteAnchorState::Lost, doc, &lang),
            "Anchor lost"
        );
        assert_eq!(
            quote_preview("  SELECT\n\t id,   name\r\nFROM dbo.users;  "),
            "SELECT id, name FROM dbo.users;"
        );
    }
}
