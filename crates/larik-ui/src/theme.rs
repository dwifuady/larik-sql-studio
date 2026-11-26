use std::sync::Arc;

use gpui_kit::component::highlighter::HighlightTheme;
use gpui_kit::component::theme::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, Window, WindowAppearance, hsla, rgb, transparent_black};
use larik_core::ThemePreference;

#[derive(Debug, Clone, Copy)]
pub struct StudioPalette {
    pub window_bg: Hsla,
    pub card_bg: Hsla,
    pub card_border: Hsla,
    pub panel_bg: Hsla,
    pub modal_bg: Hsla,
    pub subtle_hover: Hsla,
    pub subtle_fill: Hsla,
    pub tab_active_bg: Hsla,
    pub chip_active_bg: Hsla,
    pub hairline: Hsla,
    pub hairline_subtle: Hsla,
    pub row_alt: Hsla,
    pub text_primary: Hsla,
    pub text_secondary: Hsla,
    pub text_muted: Hsla,
    pub accent: Hsla,
    pub accent_hover: Hsla,
    pub accent_on: Hsla,
    pub accent_soft: Hsla,
    pub selection: Hsla,
    pub success: Hsla,
    pub warning: Hsla,
    pub danger: Hsla,
}

impl StudioPalette {
    pub fn resolve_is_dark(mode: ThemePreference, window: Option<&Window>) -> bool {
        match mode {
            ThemePreference::Dark => true,
            ThemePreference::Light => false,
            ThemePreference::System => window.is_none_or(|w| {
                matches!(
                    w.appearance(),
                    WindowAppearance::Dark | WindowAppearance::VibrantDark
                )
            }),
        }
    }

    pub fn for_mode(mode: ThemePreference) -> Self {
        let resolved = match mode {
            ThemePreference::Light => ThemePreference::Light,
            ThemePreference::Dark | ThemePreference::System => ThemePreference::Dark,
        };
        Self::build(resolved, true)
    }

    pub fn for_window(mode: ThemePreference, window: &Window) -> Self {
        let os_dark = matches!(
            window.appearance(),
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        );
        let effective = match mode {
            ThemePreference::System => {
                if os_dark {
                    ThemePreference::Dark
                } else {
                    ThemePreference::Light
                }
            }
            other => other,
        };
        let app_dark = matches!(effective, ThemePreference::Dark);
        Self::build(effective, os_dark == app_dark)
    }

    fn build(mode: ThemePreference, native_mica_matches: bool) -> Self {
        match mode {
            ThemePreference::Dark | ThemePreference::System => Self {
                window_bg: if native_mica_matches {
                    transparent_black()
                } else {
                    hsla(0.0, 0.0, 0.125, 0.96)
                },
                card_bg: hsla(0.0, 0.0, 1.0, 0.052),
                card_border: hsla(0.0, 0.0, 1.0, 0.085),
                panel_bg: hsla(0.0, 0.0, 0.08, 0.34),
                modal_bg: rgb(0x28282B).into(),
                subtle_hover: hsla(0.0, 0.0, 1.0, 0.065),
                subtle_fill: hsla(0.0, 0.0, 1.0, 0.085),
                tab_active_bg: hsla(214.0 / 360.0, 0.28, 0.62, 0.16),
                chip_active_bg: hsla(0.0, 0.0, 1.0, 0.14),
                hairline: hsla(0.0, 0.0, 1.0, 0.085),
                hairline_subtle: hsla(0.0, 0.0, 1.0, 0.055),
                row_alt: hsla(0.0, 0.0, 1.0, 0.024),
                text_primary: rgb(0xF5F5F7).into(),
                text_secondary: hsla(0.0, 0.0, 1.0, 0.76),
                text_muted: hsla(0.0, 0.0, 1.0, 0.48),
                accent: rgb(0x60CDFF).into(),
                accent_hover: rgb(0x7AD6FF).into(),
                accent_on: rgb(0x091620).into(),
                accent_soft: hsla(199.0 / 360.0, 0.95, 0.68, 0.16),
                selection: hsla(199.0 / 360.0, 0.85, 0.60, 0.25),
                success: rgb(0x34D399).into(),
                warning: rgb(0xFBBF24).into(),
                danger: rgb(0xF87171).into(),
            },
            ThemePreference::Light => Self {
                window_bg: if native_mica_matches {
                    transparent_black()
                } else {
                    hsla(0.0, 0.0, 0.953, 0.96)
                },
                card_bg: hsla(0.0, 0.0, 1.0, 0.72),
                card_border: hsla(0.0, 0.0, 0.0, 0.08),
                panel_bg: hsla(0.0, 0.0, 1.0, 0.48),
                modal_bg: rgb(0xF9F9FB).into(),
                subtle_hover: hsla(0.0, 0.0, 0.0, 0.045),
                subtle_fill: hsla(0.0, 0.0, 0.0, 0.06),
                tab_active_bg: hsla(210.0 / 360.0, 0.45, 0.50, 0.11),
                chip_active_bg: hsla(0.0, 0.0, 1.0, 0.85),
                hairline: hsla(0.0, 0.0, 0.0, 0.09),
                hairline_subtle: hsla(0.0, 0.0, 0.0, 0.055),
                row_alt: hsla(0.0, 0.0, 0.0, 0.022),
                text_primary: rgb(0x1A1A1A).into(),
                text_secondary: hsla(0.0, 0.0, 0.0, 0.72),
                text_muted: hsla(0.0, 0.0, 0.0, 0.48),
                accent: rgb(0x005FB8).into(),
                accent_hover: rgb(0x1975C5).into(),
                accent_on: rgb(0xFFFFFF).into(),
                accent_soft: hsla(209.0 / 360.0, 1.0, 0.36, 0.12),
                selection: hsla(209.0 / 360.0, 0.90, 0.45, 0.20),
                success: rgb(0x15803D).into(),
                warning: rgb(0xB45309).into(),
                danger: rgb(0xDC2626).into(),
            },
        }
    }
}

const LARIK_DARK_HIGHLIGHT_JSON: &str = r##"{
    "name": "Larik Dark",
    "appearance": "dark",
    "style": {
        "editor.background": "#00000000",
        "editor.gutter.background": "#FFFFFF05",
        "editor.foreground": "#F5F5F7",
        "editor.active_line.background": "#FFFFFF0A",
        "editor.line_number": "#7A7E8C",
        "editor.active_line_number": "#D8DBE4",
        "editor.invisible": "#FFFFFF24",
        "syntax": {
            "keyword": { "color": "#C792EA", "font_weight": 500 },
            "function": { "color": "#82AAFF" },
            "string": { "color": "#A5D6A7" },
            "string.escape": { "color": "#C3E88D" },
            "string.special": { "color": "#A5D6A7" },
            "number": { "color": "#F78C6C" },
            "boolean": { "color": "#F78C6C" },
            "constant": { "color": "#F78C6C" },
            "comment": { "color": "#8C93A6", "font_style": "italic" },
            "comment.doc": { "color": "#8C93A6", "font_style": "italic" },
            "type": { "color": "#FFCB6B" },
            "variable": { "color": "#F5F5F7" },
            "variable.special": { "color": "#FFCB6B" },
            "property": { "color": "#E2E4EC" },
            "attribute": { "color": "#FFCB6B" },
            "operator": { "color": "#9CA3B8" },
            "punctuation": { "color": "#9BA2B8" },
            "punctuation.bracket": { "color": "#B0B6C8" },
            "punctuation.delimiter": { "color": "#9BA2B8" }
        }
    }
}"##;

const LARIK_LIGHT_HIGHLIGHT_JSON: &str = r##"{
    "name": "Larik Light",
    "appearance": "light",
    "style": {
        "editor.background": "#00000000",
        "editor.gutter.background": "#00000005",
        "editor.foreground": "#1A1A1A",
        "editor.active_line.background": "#00000008",
        "editor.line_number": "#8C93A0",
        "editor.active_line_number": "#374151",
        "editor.invisible": "#0000001E",
        "syntax": {
            "keyword": { "color": "#7C3AED", "font_weight": 500 },
            "function": { "color": "#1D4ED8" },
            "string": { "color": "#047857" },
            "string.escape": { "color": "#059669" },
            "string.special": { "color": "#047857" },
            "number": { "color": "#B45309" },
            "boolean": { "color": "#B45309" },
            "constant": { "color": "#B45309" },
            "comment": { "color": "#6B7280", "font_style": "italic" },
            "comment.doc": { "color": "#6B7280", "font_style": "italic" },
            "type": { "color": "#92400E" },
            "variable": { "color": "#1A1A1A" },
            "variable.special": { "color": "#92400E" },
            "property": { "color": "#262626" },
            "attribute": { "color": "#92400E" },
            "operator": { "color": "#4B5563" },
            "punctuation": { "color": "#6B7280" },
            "punctuation.bracket": { "color": "#4B5563" },
            "punctuation.delimiter": { "color": "#6B7280" }
        }
    }
}"##;

pub fn larik_highlight_theme(mode: ThemePreference) -> Arc<HighlightTheme> {
    let json = match mode {
        ThemePreference::Dark | ThemePreference::System => LARIK_DARK_HIGHLIGHT_JSON,
        ThemePreference::Light => LARIK_LIGHT_HIGHLIGHT_JSON,
    };
    Arc::new(
        serde_json::from_str::<HighlightTheme>(json)
            .expect("embedded Larik HighlightTheme JSON must be valid"),
    )
}

pub fn apply_larik_theme(mode: ThemePreference, window: Option<&mut Window>, cx: &mut App) {
    let is_dark = StudioPalette::resolve_is_dark(mode, window.as_deref());
    let resolved = if is_dark {
        ThemePreference::Dark
    } else {
        ThemePreference::Light
    };
    let gpui_mode = if is_dark {
        ThemeMode::Dark
    } else {
        ThemeMode::Light
    };
    Theme::change(gpui_mode, window, cx);

    let p = StudioPalette::for_mode(resolved);
    let highlight = larik_highlight_theme(resolved);

    Theme::update(cx, |theme| {
        theme.colors.background = p.modal_bg;
        theme.colors.foreground = p.text_primary;
        theme.colors.border = p.hairline;
        theme.colors.window_border = p.hairline;
        theme.colors.input = p.card_border;
        theme.colors.ring = p.accent;
        theme.colors.caret = p.accent;
        theme.colors.selection = p.selection;

        theme.colors.sidebar = transparent_black();
        theme.colors.sidebar_border = p.hairline;
        theme.colors.sidebar_foreground = p.text_primary;

        theme.colors.title_bar = transparent_black();
        theme.colors.title_bar_border = transparent_black();
        theme.colors.status_bar = transparent_black();
        theme.colors.status_bar_border = p.hairline_subtle;

        theme.colors.muted = p.subtle_fill;
        theme.colors.muted_foreground = p.text_muted;
        theme.colors.secondary = p.subtle_fill;
        theme.colors.secondary_foreground = p.text_primary;
        theme.colors.secondary_hover = p.subtle_hover;
        theme.colors.secondary_active = p.chip_active_bg;

        theme.colors.popover = p.modal_bg;
        theme.colors.popover_foreground = p.text_primary;

        theme.colors.primary = p.accent;
        theme.colors.primary_hover = p.accent_hover;
        theme.colors.primary_active = p.accent;
        theme.colors.primary_foreground = p.accent_on;
        theme.colors.button_primary = p.accent;
        theme.colors.button_primary_hover = p.accent_hover;
        theme.colors.button_primary_active = p.accent;
        theme.colors.button_primary_foreground = p.accent_on;

        theme.colors.danger = p.danger;
        theme.colors.button_danger = p.danger;
        theme.colors.button_danger_foreground = rgb(0xFFFFFF).into();
        theme.colors.success = p.success;
        theme.colors.warning = p.warning;

        theme.colors.table = transparent_black();
        theme.colors.table_head = if is_dark {
            hsla(0.0, 0.0, 1.0, 0.035)
        } else {
            hsla(0.0, 0.0, 0.0, 0.03)
        };
        theme.colors.table_head_foreground = p.text_secondary;
        theme.colors.table_even = p.row_alt;
        theme.colors.table_hover = p.subtle_hover;
        theme.colors.table_active = p.accent_soft;
        theme.colors.table_active_border = p.accent;
        theme.colors.table_row_border = p.hairline_subtle;

        theme.colors.list = transparent_black();
        theme.colors.list_hover = p.subtle_hover;
        theme.colors.list_active = p.accent_soft;
        theme.colors.list_active_border = p.accent;

        theme.colors.scrollbar = transparent_black();
        theme.colors.scrollbar_thumb = if is_dark {
            hsla(0.0, 0.0, 1.0, 0.18)
        } else {
            hsla(0.0, 0.0, 0.0, 0.18)
        };
        theme.colors.scrollbar_thumb_hover = if is_dark {
            hsla(0.0, 0.0, 1.0, 0.30)
        } else {
            hsla(0.0, 0.0, 0.0, 0.30)
        };

        theme.highlight_theme = highlight;
        theme.font_family = "Segoe UI Variable".into();
        theme.mono_font_family = "Cascadia Mono".into();
        theme.mono_font_size = gpui_kit::px(13.5);
    });
    cx.refresh_windows();
}

pub fn parse_hex_color(hex: &str) -> Hsla {
    let trimmed = hex.trim().trim_start_matches('#');
    if let Ok(value) = u32::from_str_radix(trimmed, 16) {
        rgb(value).into()
    } else {
        rgb(0x60CDFF).into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_highlight_themes_deserialize() {
        let dark = larik_highlight_theme(ThemePreference::Dark);
        assert_eq!(dark.name, "Larik Dark");
        let dark_bg = dark.style.editor_background.expect("dark editor bg");
        assert_eq!(dark_bg.a, 0.0, "dark editor bg must be transparent for Mica");
        assert!(dark.style.syntax.keyword.is_some());

        let light = larik_highlight_theme(ThemePreference::Light);
        assert_eq!(light.name, "Larik Light");
        let light_bg = light.style.editor_background.expect("light editor bg");
        assert_eq!(
            light_bg.a, 0.0,
            "light editor bg must be transparent for Mica"
        );
        assert!(light.style.syntax.keyword.is_some());
    }
}
