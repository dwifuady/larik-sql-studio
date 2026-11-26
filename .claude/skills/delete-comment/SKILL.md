---
name: delete-comment
description: Audit, categorize, and eliminate code comments (Tautology, Stale, Excuse, Decision Record), file GitHub issues for excuses, move decision records into docs/adr/, and tighten .comment-ratchet.json.
---

# `/delete-comment`

Repeatable workflow for auditing and deleting code comments across `larik-sql-studio` while preserving architectural knowledge in ADRs, tracking hacky implementations as GitHub issues, and tightening the comment ratchet lint.

## Scope

Accept an optional target path, crate, or file list from the user. If no target is specified:
1. Inspect `.comment-ratchet.json` to find files with remaining comment ceilings, **or**
2. Run `cargo test -p larik-core --test comment_lint --locked` to locate any files currently failing the comment lint, **or**
3. Scan `crates/**/*.rs` for all comments (`//`, `///`, `//!`, `/* ... */`) using a lexer-aware pass (never naive regex that matches URLs or strings).

## Step 1: Inventory & Read in Context

For every comment block in scope, read the comment together with at least 5 lines of surrounding code (and verify against the live codebase when references to other types/files/behaviors appear).

## Step 2: Categorize Every Comment

Assign each comment block to exactly one category:

| Category | Definition | Required Action |
| --- | --- | --- |
| **Tautology** | Simply repeats what is already visible from the code, names, types, or test assertions (including section banners and obvious doc comments). | **Delete** the comment. |
| **Stale** | References removed code/architectures (e.g., legacy WinUI/XAML/C# classes, old file paths), inaccurate API signatures, or drifted numbers/behaviors. | **Delete** the comment. |
| **Excuse** | Rationalizes or papers over a hacky, incomplete, fragile, or workaround implementation (e.g., naive substring parsing, positional array invariants, ABA-prone pointer keys, silently swallowed errors, hardcoded layout offsets). | **Delete** the comment **and file a GitHub issue** via `gh issue create` describing the hack, why it fails, and the proper fix. Group multiple comments describing the same underlying hack into one issue. |
| **Decision Record** | Documents a genuine, non-obvious architectural, security, compatibility, performance, FFI safety, or domain invariant that cannot be trivially inferred from the code alone. | **Move** the decision into an appropriate ADR under `docs/adr/` (update an existing ADR if the theme matches, or create the next numbered `docs/adr/NNNN-<slug>.md` and add it to `docs/adr/README.md`), then **delete** the inline comment. |

## Step 3: Execute Cleanup

1. **File Excuse issues first** (before deleting the comments so line numbers and context are exact):
   ```powershell
   gh issue create --title "<crate>: <concise fix title>" --label bug --body "<background, problem, proposed fix>"
   ```
   Check `gh issue list --state open` first to avoid filing duplicates.
2. **Update or add ADRs** under `docs/adr/` for every Decision Record, keeping entries concise and indexed in `docs/adr/README.md`.
3. **Delete the comments cleanly**:
   - Preserve existing file line endings (`CRLF` vs `LF`).
   - Remove the whole line for standalone comments and collapse newly created consecutive blank lines.
   - Strip trailing horizontal whitespace left before removed inline comments.
   - Never leave dead comments or add new comments explaining the deletion.

## Step 4: Tighten the Ratchet & Verify

1. Tighten `.comment-ratchet.json` downward (never increase any file's cap):
   ```powershell
   $env:UPDATE_COMMENT_RATCHET = "1"
   cargo test -p larik-core --test comment_lint --locked
   Remove-Item Env:UPDATE_COMMENT_RATCHET
   ```
2. Verify the workspace compiles, passes the comment lint without `UPDATE_COMMENT_RATCHET`, and passes all unit tests:
   ```powershell
   cargo test -p larik-core --test comment_lint --locked
   cargo test --workspace --locked
   ```

## Step 5: Report Summary

Present a concise summary containing:
- Counts per category (Tautology, Stale, Excuse, Decision Record)
- Links to every GitHub issue filed for Excuses
- Links to every ADR created or updated
- Lines of code shed (pure comment lines + net lines after blank-line cleanup) and remaining ratchet count
