# ADR 0007: Windows 11 GPUI Shell, DWM Mica, Title Bar Hit-Testing & Workspace UX

- **Status:** Accepted
- **Date:** 2026-11-26
- **Scope:** `crates/larik-ui/build.rs`, `crates/larik-ui/src/main.rs`, `crates/larik-ui/src/window_title_bar.rs`, `crates/larik-ui/src/theme.rs`, `crates/larik-ui/src/studio.rs`, `crates/larik-core/src/lib.rs`, `crates/larik-core/src/notes.rs`

## Context

Larik SQL Studio targets a native Windows 11 Fluent desktop experience built on GPUI (`gpui-kit`). Several Win32 platform behaviors, GPUI rendering/caching constraints, and workspace interaction rules are non-obvious and must be preserved across UI refactors.

## Decisions

### 1. Windows Binary Stack Size & Executable Icon (`build.rs`)
- **16 MB main-thread stack reserve:** Windows defaults to a 1 MB main-thread stack (vs. 8 MB on macOS/Linux). Unoptimized GPUI debug render frames with deep `Div`/`StyleRefinement` chains can overflow 1 MB; `build.rs` reserves a 16 MB virtual stack (committed lazily on demand).
- **Icon resource ID `1`:** `AppIcon.ico` is embedded specifically at Win32 resource ID `1` to match `gpui-pre-windows`'s `LoadImageW(..., PCWSTR(1), IMAGE_ICON, ...)` lookup and Windows Explorer's primary executable icon convention.

### 2. Native Title Bar Hit-Testing (`window_title_bar.rs`)
- Interactive leading buttons (sidebar toggle, back arrow) are placed **outside** `WindowControlArea::Drag` as siblings rather than children.
- This ensures Win32 `WM_NCHITTEST` returns `HTCLIENT` for buttons (so clicks, hovers, and tooltips fire reliably without mouse-up being swallowed into the caption drag loop) and `HTCAPTION` exclusively for the draggable title/empty region (enabling native drag, snap, and double-click maximize).

### 3. DWM Mica, Surface Opacity & Window Placement
- **Conditional Mica transparency:** The root shell fill is transparent only when app polarity matches OS DWM Mica appearance so real Windows 11 Mica shines through; explicit Light/Dark theme overrides fall back to a calm Fluent tint.
- **Solid popovers & `BaseTheme` sync:** Floating popovers and autocomplete menus stay opaque so text remains crisp over Mica. `Theme::update` automatically re-syncs `gpui-base`'s `BaseTheme` so Editor, Input, Popover, Scrollbar, and DataTable share consistent tokens.
- **Multi-monitor placement & minimized guard:** On restore, saved display UUIDs are trusted only if that display's bounds still contain the saved window center; otherwise placement falls back to any connected display containing the center, or preserves preferred dimensions (clamped to primary visible bounds) centered on primary when off-screen/unplugged. `capture_window_placement` ignores non-finite or `< 200x200` bounds so minimized/iconic geometry never overwrites saved window placement.

### 4. Overlay Layering, Caching & Virtualization
- **Root overlay z-order:** The Workspace Switcher Flyout is hoisted out of the left rail so it never pushes sidebar elements; the Connection Settings Modal mounts above page branches so it works from both Studio and Manage Workspaces; the MCP Write Approval Modal mounts last (topmost across all pages) so security prompts are never obscured.
- **Per-result `TableState` cache:** `TableState` entities are cached by `ResultTabItem::id` so switching query tabs or result tabs is O(1) and preserves scroll position, selection, column order, and column widths.
- **Per-database schema cache & flat Schema Peek:** Schemas are cached per database with async stale-switch guards (`still_matches`). The flat virtualized `schema_rows` backing the Schema Peek drawer is rebuilt strictly on state transitions (schema, expanded set, filter query, visibility), never inside `render()`.

### 5. Workspace Tabs, Drag Targets & Statement-Anchored Notes
- **Group-owned pinning:** When a tab belongs to a tab group, pinning lives exclusively on `TabGroupState` (`tab.is_pinned` is always `false` for grouped tabs).
- **Multi-result selection:** When execution inserts multiple results, selection prefers the first tabular grid in the newly inserted slice rather than an earlier messages-only item.
- **Left-rail drag-and-drop escape hatches:** Dedicated empty drop targets sit at the bottom of both the pinned band (`10px` strip) and the unpinned scroll area (`flex_1().min_h(52.0)`) so users can always drag a tab out of a folder or pinned band.
- **Statement-anchored notes:**
  - Anchor creation/re-anchoring follows a deterministic 3-tier precedence chain: (1) non-empty editor selection, (2) dialect-aware statement under caret, (3) non-blank line under caret.
  - Revealing an anchored statement positions the caret at column 0 without creating a selection highlight; switching tabs to re-anchor a lost note seeds the caret at the note's previous start offset.
  - During single-splice text rebasing (`larik-core/src/notes.rs`), if an edit encloses the entire anchor, fingerprint quote rescue (`resolve_anchor`) runs first (handling whole-document formatting, paste, or moves). An exact in-place replacement of the anchored span re-anchors to the new text; a deletion or broader surrounding rewrite collapses the note to `Lost` rather than attaching to unrelated SQL.
