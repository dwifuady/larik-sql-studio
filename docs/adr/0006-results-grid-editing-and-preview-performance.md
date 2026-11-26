# ADR 0006: Results Grid Virtualization, Staged Row Editing & Cell Preview Performance

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-data/src/grid_actions.rs`, `crates/larik-data/src/lib.rs`, `crates/larik-ui/src/results_table.rs`, `crates/larik-ui/src/grid_actions_ui.rs`

## Context

Query result sets can contain tens of thousands of rows and multi-megabyte JSON/XML/text cells. Interactive scrolling, column reordering, staging edits, and inspecting cells in the preview pane must remain responsive on GPUI's UI thread while preserving exact original values for export and DML generation.

## Decisions

### 1. O(columns) Grid Attachment, Display Capping & Off-Thread Deallocation
- `ResultsTableDelegate` attaches a `QueryResult` in **O(columns)** time with zero upfront per-row formatting, backed by bounded lazy `SharedString` caches.
- Grid cell previews are capped at **160 characters** so DirectWrite text shaping stays instant even for `NVARCHAR(MAX)` / JSON / XML columns, while full untouched values remain in `Arc<QueryResult>` for preview and clipboard export.
- `ResultsTableDelegate::detach_for_drop` explicitly detaches owned `Arc<QueryResult>` and lazy cell caches so large deallocations happen on a background thread rather than pausing GPUI's UI thread if entity cleanup is deferred.

### 2. Column Coordinate Spaces & Pinned Row Gutter
- Column `0` (`#` row number gutter) is permanently pinned on the far left and cannot be moved or displaced.
- `GridSelection` columns are strictly in visual data-column coordinates (`0..data_cols`, excluding column `0`).
- Column reordering updates only metadata vectors and `col_order` (`col_ix - 1 -> src_col`) in O(columns) without mutating underlying `source` rows.

### 3. Staged Row Editing, Undo & Clipboard Rules
- Undo records distinguish unedited (`None`) vs. previously staged SQL `NULL` (`Some(None)`) vs. previously staged text (`Some(Some(s))`).
- Staging a value equal to the original backing cell deletes the entry from `staged_edits` so no no-op dirty indicator or redundant `UPDATE` is emitted.
- Marking a row for deletion supersedes and clears staged cell edits on that row (saving them on the undo stack) so deleted rows do not emit conflicting `UPDATE` statements.
- For multi-table `JOIN` result edits, DML planning groups edits by target table index so each modified row produces one scoped `UPDATE` per target table inside the batch transaction.
- Headerless `1x1` cell copy writes raw cell text directly to the clipboard without TSV/CSV quoting.

### 4. Cell Preview Pane Virtualization & Foreign-Key Lookup
- **O(1) frame cache:** `sync_preview_value_cache` runs on every render frame (scroll, resize drag, hover, cursor blink) and short-circuits in O(1) via key equality with zero string clones, JSON/XML parsing, or tokenization.
- **Rendering tiers:** Small plain text (`<= 4 KB` and `<= 60` lines) uses natural word-wrap; all syntax-highlighted JSON/XML and all large plain text render through GPUI's `uniform_list` virtualization.
- **Line chunking & highlight invariants:** `build_preview_lines` splits single lines exceeding `MAX_VIRTUAL_LINE_CHARS` on UTF-8 char boundaries and rebases highlight spans per chunk so minified single-line JSON/XML blobs never stall the text shaper. Syntax highlight token ranges (`grid_actions.rs`) are guaranteed to be valid UTF-8 char boundaries, non-overlapping, and strictly ascending as required by `gpui::StyledText::with_highlights`.
- **Metadata-only FK row estimates & cancellation:** `DatabaseConnection::estimate_table_row_count` uses fast catalog/metadata estimates only and never runs full-table `COUNT(*)` scans. Referenced tables with `<= 200` estimated rows render as a virtualized compact list; larger or unknown-count tables (`None`) fall back to filtered lookup cards. A dropped `oneshot` channel from `lookup_reference_rows` signifies intentional cancellation by a newer cell selection and is silently ignored.
