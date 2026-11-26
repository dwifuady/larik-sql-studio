use std::collections::{BTreeMap, HashMap};

use chrono::Utc;
use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants};
use gpui_kit::component::input::{Textarea, TextareaState};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Icon, Sizable, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    AnyElement, Context, Entity, Hsla, InteractiveElement, IntoElement, MouseButton,
    ParentElement, SharedString, StatefulInteractiveElement, Styled, Subscription, div, hsla, px,
};
use larik_core::notes::{
    NoteAnchorState, NoteKind, NoteRecord, describe_anchor, edited_label, floor_char_boundary,
    format_badge_count, line_of, quote_preview, resolve_anchor,
};
use larik_core::{SpaceState, TabState};
use larik_data::EditorLanguage;

use crate::studio::StudioView;
use crate::theme::StudioPalette;

pub const NOTES_PANE_WIDTH: f32 = 336.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NoteScope {
    #[default]
    ThisTab,
    Space,
    Unanchored,
}

#[derive(Debug, Clone)]
pub struct NoteEntry {
    pub record: NoteRecord,
    pub state: NoteAnchorState,
}

impl NoteEntry {
    pub fn from_record(mut record: NoteRecord, tab_content: &str) -> Self {
        let resolved = resolve_anchor(&record.anchor, tab_content);
        if resolved.state != NoteAnchorState::Lost
            && resolved.state != NoteAnchorState::Tab
            && (resolved.start != record.anchor.start || resolved.length != record.anchor.length)
        {
            record.anchor =
                larik_core::notes::capture_anchor(tab_content, resolved.start, resolved.length);
        }
        record.is_unanchored = resolved.state == NoteAnchorState::Lost;
        Self {
            record,
            state: resolved.state,
        }
    }

    pub fn has_quote(&self) -> bool {
        !self.record.anchor.is_tab
            && self.state != NoteAnchorState::Lost
            && !self.record.anchor.quote.trim().is_empty()
    }
}

pub struct NoteEditorPopupState {
    pub note_id: String,
    pub tab_id: String,
    pub body_input: Entity<TextareaState>,
    pub _subscription: Subscription,
}

#[derive(Debug, Clone)]
pub struct DeletedNoteUndo {
    pub record: NoteRecord,
    pub tab_id: String,
}

#[derive(Debug, Clone)]
pub struct EditorNoteMarker {
    pub start: usize,
    pub length: usize,
    pub count: usize,
    pub start_line: usize,
    pub top_note_id: String,
}

pub fn note_kind_icon(kind: NoteKind) -> IconName {
    match kind {
        NoteKind::Note => IconName::MessageSquare,
        NoteKind::Todo => IconName::Check,
        NoteKind::Decision => IconName::Star,
        NoteKind::Observation => IconName::Info,
    }
}

pub fn note_kind_color(kind: NoteKind, palette: StudioPalette) -> Hsla {
    match kind {
        NoteKind::Note => palette.text_secondary,
        NoteKind::Todo => palette.warning,
        NoteKind::Decision => palette.accent,
        NoteKind::Observation => palette.success,
    }
}

pub fn compute_editor_markers(notes: &[NoteEntry], text: &str) -> Vec<EditorNoteMarker> {
    let mut grouped: BTreeMap<(usize, usize), Vec<&NoteEntry>> = BTreeMap::new();
    for entry in notes {
        if entry.record.anchor.is_tab
            || entry.state == NoteAnchorState::Lost
            || entry.record.anchor.length == 0
        {
            continue;
        }
        grouped
            .entry((entry.record.anchor.start, entry.record.anchor.length))
            .or_default()
            .push(entry);
    }

    grouped
        .into_iter()
        .map(|((start, length), group)| {
            let top = group
                .iter()
                .max_by(|a, b| a.record.updated_at.cmp(&b.record.updated_at))
                .expect("group is non-empty");
            let safe_start = floor_char_boundary(text, start.min(text.len()));
            let start_line = line_of(text, safe_start);
            EditorNoteMarker {
                start,
                length,
                count: group.len(),
                start_line,
                top_note_id: top.record.id.clone(),
            }
        })
        .collect()
}

pub fn anchor_line_range(entry: &NoteEntry, text: &str) -> Option<(usize, usize)> {
    if entry.record.anchor.is_tab
        || entry.state == NoteAnchorState::Lost
        || entry.record.anchor.length == 0
    {
        return None;
    }
    let safe_start = floor_char_boundary(text, entry.record.anchor.start.min(text.len()));
    let raw_end = safe_start
        .saturating_add(entry.record.anchor.length)
        .min(text.len());
    let end_probe = if raw_end > safe_start {
        floor_char_boundary(text, raw_end - 1)
    } else {
        safe_start
    };
    let start_line = line_of(text, safe_start);
    let end_line = line_of(text, end_probe).max(start_line);
    Some((start_line, end_line))
}

pub fn render_notes_toolbar_button(
    tab_note_count: usize,
    notes_pane_open: bool,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let badge_text = if tab_note_count > 0 {
        Some(format_badge_count(tab_note_count))
    } else {
        None
    };

    h_flex()
        .id("toolbar-toggle-notes")
        .min_w(px(32.0))
        .h(px(28.0))
        .px_2()
        .gap_1()
        .rounded(px(6.0))
        .items_center()
        .justify_center()
        .cursor_pointer()
        .when(notes_pane_open, |el| {
            el.bg(palette.accent_soft)
                .border_1()
                .border_color(palette.accent.opacity(0.45))
        })
        .when(!notes_pane_open, |el| el.hover(|s| s.bg(palette.subtle_hover)))
        .on_click(cx.listener(|this, _, _, cx| {
            this.toggle_notes_pane(cx);
        }))
        .child(
            Icon::new(IconName::MessageSquare)
                .size(px(12.5))
                .text_color(if notes_pane_open || tab_note_count > 0 {
                    palette.accent
                } else {
                    palette.text_secondary
                }),
        )
        .when_some(badge_text, |el, count| {
            el.child(
                div()
                    .text_size(px(10.5))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(if notes_pane_open {
                        palette.text_primary
                    } else {
                        palette.accent
                    })
                    .child(count),
            )
        })
        .into_any_element()
}

pub fn render_tab_note_badge(count: usize, palette: StudioPalette) -> Option<AnyElement> {
    if count == 0 {
        return None;
    }
    Some(
        h_flex()
            .flex_shrink_0()
            .px(px(5.0))
            .py(px(1.5))
            .gap(px(3.0))
            .rounded(px(8.0))
            .bg(palette.accent_soft)
            .items_center()
            .child(
                Icon::new(IconName::MessageSquare)
                    .size(px(9.0))
                    .text_color(palette.accent),
            )
            .child(
                div()
                    .text_size(px(9.0))
                    .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                    .text_color(palette.accent)
                    .child(format_badge_count(count)),
            )
            .into_any_element(),
    )
}

#[allow(clippy::too_many_arguments)]
pub fn render_notes_pane(
    active_space: Option<&SpaceState>,
    active_tab: Option<&TabState>,
    notes_by_tab: &HashMap<String, Vec<NoteEntry>>,
    scope: NoteScope,
    active_note_id: Option<&str>,
    language: &dyn EditorLanguage,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let active_tab_id = active_tab.map(|t| t.id.as_str()).unwrap_or("");
    let now = Utc::now();

    let tab_notes = notes_by_tab
        .get(active_tab_id)
        .map(Vec::as_slice)
        .unwrap_or(&[]);
    let tab_count = tab_notes.len();
    let tab_lost_count = tab_notes
        .iter()
        .filter(|n| n.state == NoteAnchorState::Lost)
        .count();

    let (space_count, space_lost_count) = active_space.map_or((0, 0), |space| {
        let mut total = 0usize;
        let mut lost = 0usize;
        for tab in &space.tabs {
            if let Some(list) = notes_by_tab.get(&tab.id) {
                total += list.len();
                lost += list.iter().filter(|n| n.state == NoteAnchorState::Lost).count();
            }
        }
        (total, lost)
    });

    struct DisplayRow<'a> {
        entry: &'a NoteEntry,
        tab: &'a TabState,
        tab_rail_ix: usize,
        is_active_tab: bool,
    }

    let mut rows: Vec<DisplayRow<'_>> = Vec::new();
    match scope {
        NoteScope::ThisTab => {
            if let Some(tab) = active_tab {
                for entry in tab_notes {
                    rows.push(DisplayRow {
                        entry,
                        tab,
                        tab_rail_ix: 0,
                        is_active_tab: true,
                    });
                }
            }
        }
        NoteScope::Unanchored => {
            if let Some(tab) = active_tab {
                for entry in tab_notes.iter().filter(|n| n.state == NoteAnchorState::Lost) {
                    rows.push(DisplayRow {
                        entry,
                        tab,
                        tab_rail_ix: 0,
                        is_active_tab: true,
                    });
                }
            }
        }
        NoteScope::Space => {
            if let Some(space) = active_space {
                for (ix, tab) in space.tabs.iter().enumerate() {
                    if let Some(list) = notes_by_tab.get(&tab.id) {
                        for entry in list {
                            rows.push(DisplayRow {
                                entry,
                                tab,
                                tab_rail_ix: ix,
                                is_active_tab: tab.id == active_tab_id,
                            });
                        }
                    }
                }
            }
        }
    }

    rows.sort_by(|a, b| {
        let a_lost = u8::from(a.entry.state == NoteAnchorState::Lost);
        let b_lost = u8::from(b.entry.state == NoteAnchorState::Lost);
        a_lost
            .cmp(&b_lost)
            .then_with(|| (!a.is_active_tab).cmp(&(!b.is_active_tab)))
            .then_with(|| a.tab_rail_ix.cmp(&b.tab_rail_ix))
            .then_with(|| a.entry.record.anchor.start.cmp(&b.entry.record.anchor.start))
            .then_with(|| a.entry.record.created_at.cmp(&b.entry.record.created_at))
    });

    let visible_count = rows.len();

    let (summary_total, summary_lost) = match scope {
        NoteScope::Space => (space_count, space_lost_count),
        NoteScope::ThisTab | NoteScope::Unanchored => (tab_count, tab_lost_count),
    };
    let summary_anchored = summary_total.saturating_sub(summary_lost);
    let summary_text = if summary_lost == 0 {
        format!("{summary_anchored} anchored")
    } else if summary_lost == 1 {
        format!("{summary_anchored} anchored · 1 needs attention")
    } else {
        format!("{summary_anchored} anchored · {summary_lost} need attention")
    };

    let mono_font = cx.theme().mono_font_family.clone();

    v_flex()
        .id("notes-pane")
        .w(px(NOTES_PANE_WIDTH))
        .h_full()
        .flex_shrink_0()
        .bg(palette.panel_bg)
        .border_l_1()
        .border_color(palette.hairline)
        .overflow_hidden()
        .child(
            h_flex()
                .h(px(46.0))
                .px_3p5()
                .border_b_1()
                .border_color(palette.hairline)
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(palette.text_primary)
                                .child("Notes"),
                        )
                        .child(
                            div()
                                .px(px(6.0))
                                .py(px(1.0))
                                .rounded(px(9.0))
                                .bg(palette.hairline)
                                .text_size(px(10.0))
                                .text_color(palette.text_secondary)
                                .child(visible_count.to_string()),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1()
                        .child(
                            Button::new("notes-pane-new")
                                .ghost()
                                .xsmall()
                                .w(px(26.0))
                                .h(px(26.0))
                                .icon(IconName::Plus)
                                .tooltip("Add a note on the statement under the caret (Ctrl+Alt+N)")
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.create_note_at_caret(window, cx);
                                })),
                        )
                        .child(
                            Button::new("notes-pane-close")
                                .ghost()
                                .xsmall()
                                .w(px(26.0))
                                .h(px(26.0))
                                .icon(IconName::X)
                                .tooltip("Close notes")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.close_notes_pane(cx);
                                })),
                        ),
                ),
        )
        .child(
            h_flex()
                .px_2p5()
                .pt_2()
                .pb_1p5()
                .gap(px(3.0))
                .items_center()
                .child(render_scope_pill(
                    "notes-scope-tab",
                    "This tab",
                    tab_count,
                    scope == NoteScope::ThisTab,
                    NoteScope::ThisTab,
                    palette,
                    cx,
                ))
                .child(render_scope_pill(
                    "notes-scope-space",
                    "All",
                    space_count,
                    scope == NoteScope::Space,
                    NoteScope::Space,
                    palette,
                    cx,
                ))
                .child(render_scope_pill(
                    "notes-scope-lost",
                    "Unanchored",
                    tab_lost_count,
                    scope == NoteScope::Unanchored,
                    NoteScope::Unanchored,
                    palette,
                    cx,
                )),
        )
        .child(if rows.is_empty() {
            let (empty_title, empty_detail) = match scope {
                NoteScope::ThisTab => (
                    "No notes in this tab",
                    "Select a statement and press Ctrl+Alt+N.",
                ),
                NoteScope::Space => (
                    "No notes in this workspace",
                    "Notes left on any tab in this workspace appear here.",
                ),
                NoteScope::Unanchored => (
                    "Every note is anchored",
                    "Nothing needs a new anchor.",
                ),
            };
            v_flex()
                .flex_1()
                .min_h_0()
                .w_full()
                .px(px(18.0))
                .pt(px(24.0))
                .gap(px(7.0))
                .child(
                    Icon::new(IconName::MessageSquare)
                        .size(px(20.0))
                        .text_color(palette.text_muted),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(palette.text_primary)
                        .child(empty_title),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(palette.text_secondary)
                        .child(empty_detail),
                )
                .into_any_element()
        } else {
            v_flex()
                .id("notes-cards-scroll")
                .flex_1()
                .min_h_0()
                .w_full()
                .overflow_y_scroll()
                .px_2p5()
                .pt(px(2.0))
                .pb_2()
                .gap(px(6.0))
                .children(rows.into_iter().map(|row| {
                    let is_active = active_note_id == Some(row.entry.record.id.as_str());
                    let anchor_text = describe_anchor(
                        &row.entry.record.anchor,
                        row.entry.state,
                        &row.tab.content,
                        language,
                    );
                    render_note_card(
                        row.entry,
                        &anchor_text,
                        if scope == NoteScope::Space {
                            Some(row.tab.title.as_str())
                        } else {
                            None
                        },
                        is_active,
                        now,
                        mono_font.clone(),
                        palette,
                        cx,
                    )
                }))
                .into_any_element()
        })
        .child(
            h_flex()
                .h(px(38.0))
                .px_3p5()
                .border_t_1()
                .border_color(palette.hairline)
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    div()
                        .text_size(px(10.0))
                        .text_color(palette.text_muted)
                        .truncate()
                        .child(summary_text),
                )
                .child(
                    h_flex()
                        .id("notes-footer-space-btn")
                        .px_1p5()
                        .py(px(3.0))
                        .gap_1()
                        .rounded(px(5.0))
                        .items_center()
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.subtle_hover))
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.set_notes_scope(NoteScope::Space, cx);
                        }))
                        .child(
                            div()
                                .text_size(px(10.5))
                                .text_color(palette.text_secondary)
                                .child("Space notes"),
                        )
                        .child(
                            Icon::new(IconName::ChevronRight)
                                .size(px(10.0))
                                .text_color(palette.text_muted),
                        ),
                ),
        )
        .into_any_element()
}

fn render_scope_pill(
    id: &'static str,
    label: &'static str,
    count: usize,
    active: bool,
    target_scope: NoteScope,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    h_flex()
        .id(id)
        .h(px(24.0))
        .px(px(9.0))
        .gap(px(5.0))
        .rounded(px(6.0))
        .items_center()
        .cursor_pointer()
        .when(active, |el| el.bg(palette.accent_soft))
        .when(!active, |el| el.hover(|s| s.bg(palette.subtle_hover)))
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_notes_scope(target_scope, cx);
        }))
        .child(
            div()
                .text_size(px(11.0))
                .font_weight(if active {
                    gpui_kit::FontWeight::MEDIUM
                } else {
                    gpui_kit::FontWeight::NORMAL
                })
                .text_color(if active {
                    palette.text_primary
                } else {
                    palette.text_muted
                })
                .child(label),
        )
        .child(
            div()
                .text_size(px(9.5))
                .text_color(if active {
                    palette.text_secondary
                } else {
                    palette.text_muted
                })
                .child(count.to_string()),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_note_card(
    entry: &NoteEntry,
    anchor_text: &str,
    tab_title: Option<&str>,
    is_active: bool,
    now: chrono::DateTime<Utc>,
    mono_font: SharedString,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let note_id = entry.record.id.clone();
    let tab_id = entry.record.tab_id.clone();
    let note_id_reanchor = note_id.clone();
    let tab_id_reanchor = tab_id.clone();
    let kind = entry.record.kind;
    let kind_color = note_kind_color(kind, palette);
    let is_lost = entry.state == NoteAnchorState::Lost;
    let has_quote = entry.has_quote();
    let quote_text = if has_quote {
        quote_preview(&entry.record.anchor.quote)
    } else {
        String::new()
    };

    let body_trimmed = entry.record.body.trim();
    let has_body = !body_trimmed.is_empty();
    let body_display = if has_body {
        let mut lines: Vec<&str> = body_trimmed.lines().take(4).collect();
        let truncated = body_trimmed.lines().count() > 4;
        let mut s = lines.join("\n");
        if truncated {
            s.push('…');
        }
        let _ = &mut lines;
        s
    } else {
        "Empty note".to_string()
    };

    let meta_text = if is_lost {
        "The text this was attached to is gone".to_string()
    } else {
        format!("Edited {}", edited_label(&entry.record.updated_at, now))
    };

    v_flex()
        .id(SharedString::from(format!("note-card-{}", entry.record.id)))
        .relative()
        .w_full()
        .px(px(10.0))
        .py(px(8.0))
        .gap(px(6.0))
        .rounded(px(8.0))
        .bg(if is_active {
            palette.accent_soft
        } else {
            palette.card_bg
        })
        .border_1()
        .border_color(if is_active {
            palette.accent
        } else {
            palette.hairline
        })
        .cursor_pointer()
        .when(!is_active, |el| el.hover(|s| s.bg(palette.subtle_hover)))
        .on_click(cx.listener(move |this, _, window, cx| {
            this.open_note_from_card(tab_id.clone(), note_id.clone(), window, cx);
        }))
        .when(is_active, |el| {
            el.child(
                div()
                    .absolute()
                    .left(px(0.0))
                    .top(px(6.0))
                    .bottom(px(6.0))
                    .w(px(2.5))
                    .rounded(px(1.0))
                    .bg(palette.accent),
            )
        })
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_2()
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            Icon::new(note_kind_icon(kind))
                                .size(px(10.5))
                                .text_color(kind_color),
                        )
                        .child(
                            div()
                                .text_size(px(10.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(kind_color)
                                .child(kind.label()),
                        ),
                )
                .when(is_lost, |row| {
                    row.child(
                        div()
                            .id(SharedString::from(format!(
                                "note-reanchor-{}",
                                entry.record.id
                            )))
                            .px(px(6.0))
                            .py(px(1.5))
                            .rounded(px(4.0))
                            .bg(palette.accent_soft)
                            .hover(|s| s.bg(palette.accent.opacity(0.28)))
                            .text_size(px(10.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(palette.accent)
                            .cursor_pointer()
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                cx.stop_propagation();
                                this.reanchor_note(
                                    tab_id_reanchor.clone(),
                                    note_id_reanchor.clone(),
                                    window,
                                    cx,
                                );
                            }))
                            .child("Re-anchor"),
                    )
                }),
        )
        .when(has_quote, |col| {
            col.child(
                div()
                    .w_full()
                    .px(px(7.0))
                    .py(px(3.0))
                    .rounded_r(px(4.0))
                    .bg(palette.hairline_subtle)
                    .border_l_2()
                    .border_color(palette.hairline)
                    .font_family(mono_font)
                    .text_size(px(10.5))
                    .text_color(palette.text_secondary)
                    .truncate()
                    .child(quote_text),
            )
        })
        .child(
            div()
                .w_full()
                .text_size(px(12.0))
                .line_height(px(17.0))
                .text_color(if has_body {
                    palette.text_primary
                } else {
                    palette.text_muted
                })
                .when(!has_body, |d| d.italic())
                .child(body_display),
        )
        .child(
            h_flex()
                .w_full()
                .items_center()
                .gap(px(5.0))
                .text_size(px(10.0))
                .text_color(palette.text_muted)
                .child(
                    Icon::new(IconName::Pin)
                        .size(px(9.5))
                        .flex_shrink_0()
                        .text_color(if is_lost {
                            palette.warning
                        } else {
                            palette.text_muted
                        }),
                )
                .child(
                    div()
                        .flex_shrink_0()
                        .text_color(if is_lost {
                            palette.warning
                        } else {
                            palette.text_muted
                        })
                        .child(anchor_text.to_string()),
                )
                .child(div().flex_shrink_0().child("·"))
                .child(div().flex_1().min_w_0().truncate().child(meta_text))
                .when_some(tab_title, |row, title| {
                    row.child(
                        div()
                            .flex_shrink_0()
                            .max_w(px(96.0))
                            .px(px(5.0))
                            .py(px(1.0))
                            .rounded(px(4.0))
                            .bg(palette.subtle_fill)
                            .text_size(px(9.5))
                            .text_color(palette.text_secondary)
                            .truncate()
                            .child(title.to_string()),
                    )
                }),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
pub fn render_editor_note_overlays(
    markers: &[EditorNoteMarker],
    active_line_range: Option<(usize, usize)>,
    line_height_px: f32,
    scroll_offset_y: f32,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> Vec<AnyElement> {
    let mut elements = Vec::new();
    let top_inset = 8.0_f32;
    let lh = line_height_px.max(16.0);

    if let Some((start_line, end_line)) = active_line_range {
        let top = top_inset + (start_line.saturating_sub(1) as f32) * lh + scroll_offset_y;
        let line_count = end_line.saturating_sub(start_line) + 1;
        let height = (line_count as f32 * lh).max(lh);
        if top + height > 0.0 {
            let clamped_top = top.max(2.0);
            let visible_h = (top + height - clamped_top).max(4.0);
            elements.push(
                div()
                    .absolute()
                    .left(px(1.5))
                    .top(px(clamped_top))
                    .w(px(3.0))
                    .h(px(visible_h))
                    .rounded(px(2.0))
                    .bg(palette.accent)
                    .into_any_element(),
            );
        }
    }

    for marker in markers {
        let line_y = top_inset + (marker.start_line.saturating_sub(1) as f32) * lh + scroll_offset_y;
        let badge_h = 22.0_f32;
        let top = line_y + ((lh - badge_h) * 0.5).max(0.0);
        if top + badge_h < 0.0 {
            continue;
        }
        let note_id = marker.top_note_id.clone();
        let count = marker.count;
        elements.push(
            h_flex()
                .id(SharedString::from(format!(
                    "editor-note-marker-{}-{}",
                    marker.start, marker.length
                )))
                .absolute()
                .right(px(16.0))
                .top(px(top))
                .h(px(badge_h))
                .min_w(px(24.0))
                .px(px(6.0))
                .gap(px(3.0))
                .rounded(px(6.0))
                .bg(palette.accent_soft)
                .border_1()
                .border_color(palette.accent)
                .items_center()
                .justify_center()
                .cursor_pointer()
                .hover(|s| s.bg(palette.accent.opacity(0.28)))
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .on_click(cx.listener(move |this, _, window, cx| {
                    cx.stop_propagation();
                    this.open_note_by_id(note_id.clone(), window, cx);
                }))
                .child(
                    Icon::new(IconName::MessageSquare)
                        .size(px(10.5))
                        .text_color(palette.text_primary),
                )
                .when(count > 1, |el| {
                    el.child(
                        div()
                            .text_size(px(10.0))
                            .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                            .text_color(palette.text_primary)
                            .child(count.to_string()),
                    )
                })
                .into_any_element(),
        );
    }

    elements
}

pub fn render_note_editor_popover(
    entry: &NoteEntry,
    anchor_text: &str,
    popup: &NoteEditorPopupState,
    top_offset_px: f32,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let mono_font = cx.theme().mono_font_family.clone();
    let active_kind = entry.record.kind;
    let has_quote = entry.has_quote();
    let quote_text = if has_quote {
        quote_preview(&entry.record.anchor.quote)
    } else {
        String::new()
    };
    let is_lost = entry.state == NoteAnchorState::Lost;
    let clamped_top = top_offset_px.clamp(12.0, 260.0);

    div()
        .id("note-popover-backdrop")
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.01))
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                this.close_note_popover(false, None, cx);
            }),
        )
        .child(
            v_flex()
                .id("note-popover-card")
                .absolute()
                .right(px(56.0))
                .top(px(clamped_top))
                .w(px(384.0))
                .max_w_full()
                .rounded(px(10.0))
                .bg(palette.modal_bg)
                .border_1()
                .border_color(palette.card_border)
                .shadow_2xl()
                .overflow_hidden()
                .occlude()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    v_flex()
                        .px_3()
                        .pt_2p5()
                        .pb_2p5()
                        .gap_2()
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .gap_1()
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(3.0))
                                        .children(NoteKind::ALL.into_iter().map(|kind| {
                                            let selected = kind == active_kind;
                                            let k_color = note_kind_color(kind, palette);
                                            h_flex()
                                                .id(SharedString::from(format!(
                                                    "note-kind-{}",
                                                    kind.as_str()
                                                )))
                                                .h(px(24.0))
                                                .px(px(7.0))
                                                .gap(px(4.0))
                                                .rounded(px(6.0))
                                                .items_center()
                                                .border_1()
                                                .border_color(if selected {
                                                    palette.accent
                                                } else {
                                                    gpui_kit::transparent_black()
                                                })
                                                .when(selected, |el| el.bg(palette.accent_soft))
                                                .when(!selected, |el| {
                                                    el.hover(|s| s.bg(palette.subtle_hover))
                                                })
                                                .cursor_pointer()
                                                .on_click(cx.listener(move |this, _, _, cx| {
                                                    this.set_active_note_kind(kind, cx);
                                                }))
                                                .child(
                                                    Icon::new(note_kind_icon(kind))
                                                        .size(px(10.0))
                                                        .text_color(if selected {
                                                            k_color
                                                        } else {
                                                            palette.text_muted
                                                        }),
                                                )
                                                .child(
                                                    div()
                                                        .text_size(px(11.0))
                                                        .font_weight(if selected {
                                                            gpui_kit::FontWeight::SEMIBOLD
                                                        } else {
                                                            gpui_kit::FontWeight::NORMAL
                                                        })
                                                        .text_color(if selected {
                                                            palette.text_primary
                                                        } else {
                                                            palette.text_muted
                                                        })
                                                        .child(kind.label()),
                                                )
                                        })),
                                )
                                .child(
                                    h_flex()
                                        .items_center()
                                        .gap(px(2.0))
                                        .child(
                                            Button::new("note-popover-delete")
                                                .ghost()
                                                .xsmall()
                                                .w(px(26.0))
                                                .h(px(26.0))
                                                .icon(IconName::Trash)
                                                .tooltip("Delete note")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.delete_active_popover_note(window, cx);
                                                })),
                                        )
                                        .child(
                                            Button::new("note-popover-close")
                                                .ghost()
                                                .xsmall()
                                                .w(px(26.0))
                                                .h(px(26.0))
                                                .icon(IconName::X)
                                                .tooltip("Close (Esc)")
                                                .on_click(cx.listener(|this, _, window, cx| {
                                                    this.close_note_popover(true, Some(window), cx);
                                                })),
                                        ),
                                ),
                        )
                        .when(has_quote, |col| {
                            col.child(
                                div()
                                    .w_full()
                                    .px(px(7.0))
                                    .py(px(4.0))
                                    .rounded_r(px(4.0))
                                    .bg(palette.hairline_subtle)
                                    .border_l_2()
                                    .border_color(palette.accent)
                                    .font_family(mono_font)
                                    .text_size(px(11.0))
                                    .text_color(palette.text_secondary)
                                    .truncate()
                                    .child(quote_text),
                            )
                        })
                        .child(
                            h_flex()
                                .items_center()
                                .gap(px(5.0))
                                .child(
                                    Icon::new(IconName::Pin)
                                        .size(px(9.5))
                                        .text_color(if is_lost {
                                            palette.warning
                                        } else {
                                            palette.text_muted
                                        }),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(if is_lost {
                                            palette.warning
                                        } else {
                                            palette.text_muted
                                        })
                                        .child(anchor_text.to_string()),
                                ),
                        )
                        .child(
                            div()
                                .w_full()
                                .rounded(px(6.0))
                                .bg(palette.subtle_fill)
                                .border_1()
                                .border_color(palette.hairline)
                                .overflow_hidden()
                                .child(
                                    Textarea::new(&popup.body_input)
                                        .appearance(false)
                                        .bordered(false)
                                        .h(px(116.0)),
                                ),
                        ),
                )
                .child(
                    h_flex()
                        .h(px(32.0))
                        .px_3()
                        .bg(palette.hairline_subtle)
                        .border_t_1()
                        .border_color(palette.hairline_subtle)
                        .items_center()
                        .justify_between()
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1p5()
                                .child(
                                    Icon::new(IconName::Check)
                                        .size(px(10.5))
                                        .text_color(palette.success),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(palette.text_muted)
                                        .child("Saved automatically"),
                                ),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_1()
                                .child(
                                    div()
                                        .px(px(4.0))
                                        .py(px(1.0))
                                        .rounded(px(3.0))
                                        .bg(palette.hairline)
                                        .text_size(px(9.0))
                                        .text_color(palette.text_secondary)
                                        .child("Esc"),
                                )
                                .child(
                                    div()
                                        .text_size(px(10.0))
                                        .text_color(palette.text_muted)
                                        .child("close"),
                                ),
                        ),
                ),
        )
        .into_any_element()
}
