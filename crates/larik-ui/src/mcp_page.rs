use std::path::{Path, PathBuf};
use std::time::Instant;

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::theme::ActiveTheme;
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, InteractiveElement as _, IntoElement, MouseButton, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, hsla, px, rgb,
    transparent_black,
};
use larik_core::mcp::{
    McpAccess, McpApprovalDecision, McpApprovalRequest, McpLogRecord, McpSpaceRecord,
    mcp_log_status, mcp_pipe,
};
use larik_core::store::StudioStore;
use larik_data::ProviderCatalog;
use tokio::sync::oneshot;

use crate::studio::StudioView;
use crate::theme::StudioPalette;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AgentClientKind {
    ClaudeCode,
    Cursor,
    VsCode,
    Codex,
}

impl AgentClientKind {
    pub const ALL: [AgentClientKind; 4] = [
        AgentClientKind::ClaudeCode,
        AgentClientKind::Cursor,
        AgentClientKind::VsCode,
        AgentClientKind::Codex,
    ];

    pub fn id(self) -> &'static str {
        match self {
            Self::ClaudeCode => "claude",
            Self::Cursor => "cursor",
            Self::VsCode => "vscode",
            Self::Codex => "codex",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Self::ClaudeCode => "Claude Code",
            Self::Cursor => "Cursor",
            Self::VsCode => "VS Code",
            Self::Codex => "Codex",
        }
    }
}

pub struct PendingMcpApproval {
    pub seq: u64,
    pub request: McpApprovalRequest,
    pub reply: Option<oneshot::Sender<McpApprovalDecision>>,
    pub expires_at: Instant,
}

impl PendingMcpApproval {
    pub fn remaining_secs(&self) -> u64 {
        self.expires_at
            .saturating_duration_since(Instant::now())
            .as_secs()
    }
}

pub struct McpSettingsState {
    pub enabled: bool,
    pub spaces: Vec<McpSpaceRecord>,
    pub selected_client: AgentClientKind,
    pub activity: Vec<McpLogRecord>,
    pub copied_command_toast: bool,
    pub confirming_clear_activity: bool,
    pub server_path: Option<PathBuf>,
    pub custom_state_dir: Option<PathBuf>,
}

impl McpSettingsState {
    pub fn load(store: &StudioStore) -> Self {
        let enabled = store.is_mcp_enabled();
        let spaces = store
            .get_mcp_spaces()
            .into_iter()
            .filter(|s| s.has_connection)
            .collect();
        let activity = store.get_mcp_log(30);
        let custom_state_dir = std::env::var("LARIK_STATE_DIR")
            .ok()
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty())
            .map(PathBuf::from);
        Self {
            enabled,
            spaces,
            selected_client: AgentClientKind::ClaudeCode,
            activity,
            copied_command_toast: false,
            confirming_clear_activity: false,
            server_path: find_mcp_server_path(),
            custom_state_dir,
        }
    }

    pub fn refresh(&mut self, store: &StudioStore) {
        self.enabled = store.is_mcp_enabled();
        self.spaces = store
            .get_mcp_spaces()
            .into_iter()
            .filter(|s| s.has_connection)
            .collect();
        self.activity = store.get_mcp_log(30);
    }

    pub fn refresh_activity(&mut self, store: &StudioStore) {
        self.activity = store.get_mcp_log(30);
    }

    pub fn current_command(&self) -> String {
        format_agent_command(
            self.selected_client,
            self.server_path.as_deref(),
            self.custom_state_dir.as_deref(),
        )
    }
}

pub fn find_mcp_server_path() -> Option<PathBuf> {
    let exe_name = if cfg!(windows) {
        "larik-mcp.exe"
    } else {
        "larik-mcp"
    };

    let exe_dir = std::env::current_exe().ok()?.parent()?.to_path_buf();
    let candidates = [
        exe_dir.join("mcp").join(exe_name),
        exe_dir.join(exe_name),
    ];
    for candidate in candidates {
        if candidate.is_file() {
            return Some(candidate);
        }
    }

    let mut cursor = exe_dir.as_path();
    for _ in 0..5 {
        for profile in ["debug", "release"] {
            let candidate = cursor.join("target").join(profile).join(exe_name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
        cursor = cursor.parent()?;
    }

    None
}

pub fn format_agent_command(
    client: AgentClientKind,
    server_path: Option<&Path>,
    state_dir: Option<&Path>,
) -> String {
    let command_str = server_path
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| "larik-mcp".to_string());

    let quote_cli = |val: &str| -> String {
        if val.contains(' ') {
            format!("\"{val}\"")
        } else {
            val.to_string()
        }
    };

    let cli_target = match state_dir {
        Some(dir) => format!(
            "{} --state {}",
            quote_cli(&command_str),
            quote_cli(&dir.display().to_string())
        ),
        None => quote_cli(&command_str),
    };

    let command_json = {
        let cmd_val = serde_json::to_string(&command_str).unwrap_or_else(|_| "\"larik-mcp\"".into());
        match state_dir {
            Some(dir) => {
                let dir_val = serde_json::to_string(&dir.display().to_string())
                    .unwrap_or_else(|_| "\"\"".into());
                format!("\"command\":{cmd_val},\"args\":[\"--state\",{dir_val}]")
            }
            None => format!("\"command\":{cmd_val}"),
        }
    };

    match client {
        AgentClientKind::ClaudeCode => format!("claude mcp add larik -- {cli_target}"),
        AgentClientKind::Cursor => {
            format!("{{ \"mcpServers\": {{ \"larik\": {{ {command_json} }} }} }}")
        }
        AgentClientKind::VsCode => {
            format!("code --add-mcp '{{\"name\":\"larik\",\"type\":\"stdio\",{command_json}}}'")
        }
        AgentClientKind::Codex => format!("codex mcp add larik -- {cli_target}"),
    }
}

pub fn render_ai_agents_section(
    state: &McpSettingsState,
    catalog: &ProviderCatalog,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> impl IntoElement {
    let coral = rgb(0xF27B5E);
    let mono_font = cx.theme().mono_font_family.clone();
    let enabled = state.enabled;
    let command_text = state.current_command();
    let server_note = if state.server_path.is_some() {
        "The command points at this build's MCP server (`larik-mcp`). Credentials stay in Windows Credential Manager."
    } else {
        "Build `larik-mcp` (`cargo build -p larik-mcp`) so the server binary sits beside `larik-sql-studio`, or place `larik-mcp` on PATH."
    };

    div()
        .id("settings-agents-scroll")
        .relative()
        .size_full()
        .overflow_y_scroll()
        .child(
            v_flex()
                .max_w(px(760.0))
                .gap_3p5()
                .pb_6()
                .child(
                    v_flex()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(20.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(palette.text_primary)
                                .child("AI agents"),
                        )
                        .child(
                            div()
                                .text_size(px(12.0))
                                .text_color(palette.text_muted)
                                .child(
                                    "Claude Code, Cursor, VS Code and Codex can use the connections you already saved. Credentials stay in Windows Credential Manager: the agent asks the MCP server, and the MCP server asks Larik what it may read.",
                                ),
                        ),
                )
                .child(
                    v_flex()
                        .p_5()
                        .gap_3()
                        .rounded(px(8.0))
                        .bg(palette.card_bg)
                        .border_1()
                        .border_color(palette.card_border)
                        .child(
                            h_flex()
                                .items_center()
                                .justify_between()
                                .gap_4()
                                .child(
                                    v_flex()
                                        .gap_0p5()
                                        .child(
                                            div()
                                                .text_size(px(14.0))
                                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                                .text_color(palette.text_primary)
                                                .child("Allow AI agents"),
                                        )
                                        .child(
                                            div()
                                                .text_size(px(11.5))
                                                .text_color(palette.text_muted)
                                                .child(if enabled {
                                                    "On — the agents you configure below can read the workspaces you expose, even while Larik is closed."
                                                } else {
                                                    "Off — no agent can connect. Nothing is listed and nothing is served."
                                                }),
                                        ),
                                )
                                .child(render_toggle_switch(
                                    "agents-master-toggle",
                                    enabled,
                                    palette,
                                    cx,
                                )),
                        ),
                )
                .when(enabled, |col| {
                    col
                        .child(render_workspaces_card(state, catalog, palette, cx))
                        .child(render_connect_card(
                            state,
                            &command_text,
                            server_note,
                            &mono_font,
                            coral,
                            palette,
                            cx,
                        ))
                        .child(render_activity_card(state, palette, cx))
                }),
        )
        .when(state.confirming_clear_activity, |root| {
            root.child(render_clear_activity_confirm(palette, cx))
        })
}

fn render_toggle_switch(
    id: &'static str,
    checked: bool,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let coral = rgb(0xF27B5E);
    h_flex()
        .id(id)
        .items_center()
        .gap_2()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_mcp_enabled(!checked, cx);
        }))
        .child(
            div()
                .text_size(px(12.0))
                .text_color(if checked {
                    palette.text_primary
                } else {
                    palette.text_muted
                })
                .child(if checked { "On" } else { "Off" }),
        )
        .child(
            div()
                .w(px(40.0))
                .h(px(22.0))
                .rounded_full()
                .p(px(2.5))
                .flex()
                .items_center()
                .when(checked, |d| d.justify_end().bg(coral))
                .when(!checked, |d| {
                    d.justify_start()
                        .bg(palette.subtle_fill)
                        .border_1()
                        .border_color(palette.card_border)
                })
                .child(
                    div()
                        .size(px(15.0))
                        .rounded_full()
                        .bg(if checked {
                            rgb(0x1C1412).into()
                        } else {
                            palette.text_secondary
                        }),
                ),
        )
        .into_any_element()
}

fn render_workspaces_card(
    state: &McpSettingsState,
    catalog: &ProviderCatalog,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    v_flex()
        .p_5()
        .gap_3p5()
        .rounded(px(8.0))
        .bg(palette.card_bg)
        .border_1()
        .border_color(palette.card_border)
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(palette.text_primary)
                        .child("Workspaces agents may use"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(palette.text_muted)
                        .child(
                            "Read only is enforced by the database, not by trusting what the agent writes.",
                        ),
                ),
        )
        .child(if state.spaces.is_empty() {
            div()
                .py_2()
                .text_size(px(12.0))
                .text_color(palette.text_muted)
                .child("A workspace needs a saved connection before an agent can use it.")
                .into_any_element()
        } else {
            v_flex()
                .gap_2()
                .children(state.spaces.iter().map(|space| {
                    let provider_label = catalog
                        .find(&space.provider_id)
                        .map(|p| p.display_name().to_string())
                        .unwrap_or_else(|| format!("{} (not installed)", space.provider_id));
                    render_workspace_access_row(space, provider_label, palette, cx)
                }))
                .into_any_element()
        })
        .child(
            div()
                .text_size(px(11.5))
                .text_color(palette.text_muted)
                .child(
                    "Writes ask you first: the statement appears in Larik, and the agent waits for your answer. With Larik closed, writes are refused.",
                ),
        )
        .into_any_element()
}

fn render_workspace_access_row(
    space: &McpSpaceRecord,
    provider_label: String,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let space_id = space.id.clone();
    let current = space.access;

    h_flex()
        .px_3()
        .py_2()
        .rounded(px(6.0))
        .bg(palette.subtle_fill)
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(1.5))
                .child(
                    div()
                        .text_size(px(13.0))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(space.name.clone()),
                )
                .child(
                    div()
                        .text_size(px(10.5))
                        .text_color(palette.text_muted)
                        .truncate()
                        .child(provider_label),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .p(px(2.0))
                .gap(px(2.0))
                .rounded(px(6.0))
                .bg(palette.card_bg)
                .border_1()
                .border_color(palette.card_border)
                .children(McpAccess::ALL.into_iter().map(|level| {
                    let selected = current == level;
                    let sid = space_id.clone();
                    let active_bg = match level {
                        McpAccess::Off => palette.subtle_hover,
                        McpAccess::Read => hsla(210.0 / 360.0, 0.72, 0.52, 0.22),
                        McpAccess::Write => hsla(14.0 / 360.0, 0.85, 0.62, 0.24),
                    };
                    let active_text = match level {
                        McpAccess::Off => palette.text_primary,
                        McpAccess::Read => palette.text_primary,
                        McpAccess::Write => rgb(0xF27B5E).into(),
                    };

                    div()
                        .id(SharedString::from(format!(
                            "mcp-access-{}-{}",
                            sid,
                            level.as_storage()
                        )))
                        .px_2p5()
                        .py(px(4.0))
                        .rounded(px(4.0))
                        .text_size(px(11.5))
                        .font_weight(if selected {
                            gpui_kit::FontWeight::MEDIUM
                        } else {
                            gpui_kit::FontWeight::NORMAL
                        })
                        .cursor_pointer()
                        .when(selected, |d| d.bg(active_bg).text_color(active_text))
                        .when(!selected, |d| {
                            d.bg(transparent_black())
                                .text_color(palette.text_muted)
                                .hover(|s| {
                                    s.bg(palette.subtle_hover)
                                        .text_color(palette.text_secondary)
                                })
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.set_space_mcp_access(sid.clone(), level, cx);
                        }))
                        .child(level.label())
                })),
        )
        .into_any_element()
}

#[allow(clippy::too_many_arguments)]
fn render_connect_card(
    state: &McpSettingsState,
    command_text: &str,
    server_note: &'static str,
    mono_font: &SharedString,
    coral: gpui_kit::Rgba,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let selected = state.selected_client;
    let cmd_for_copy = command_text.to_string();

    v_flex()
        .p_5()
        .gap_3p5()
        .rounded(px(8.0))
        .bg(palette.card_bg)
        .border_1()
        .border_color(palette.card_border)
        .child(
            v_flex()
                .gap_0p5()
                .child(
                    div()
                        .text_size(px(14.0))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(palette.text_primary)
                        .child("Connect an agent"),
                )
                .child(
                    div()
                        .text_size(px(11.5))
                        .text_color(palette.text_muted)
                        .child("Run this once in the agent's terminal. There is no port and no token to paste."),
                ),
        )
        .child(
            h_flex()
                .items_center()
                .gap_1p5()
                .children(AgentClientKind::ALL.into_iter().map(|client| {
                    let is_active = client == selected;
                    div()
                        .id(SharedString::from(format!("mcp-client-{}", client.id())))
                        .px_3()
                        .py(px(5.0))
                        .rounded(px(6.0))
                        .border_1()
                        .border_color(if is_active {
                            coral.into()
                        } else {
                            palette.card_border
                        })
                        .bg(if is_active {
                            hsla(14.0 / 360.0, 0.85, 0.62, 0.16)
                        } else {
                            palette.subtle_fill
                        })
                        .text_size(px(12.0))
                        .font_weight(if is_active {
                            gpui_kit::FontWeight::MEDIUM
                        } else {
                            gpui_kit::FontWeight::NORMAL
                        })
                        .text_color(if is_active {
                            palette.text_primary
                        } else {
                            palette.text_secondary
                        })
                        .cursor_pointer()
                        .hover(|s| s.bg(palette.subtle_hover))
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.select_mcp_client(client, cx);
                        }))
                        .child(client.label())
                })),
        )
        .child(
            div()
                .w_full()
                .p_3()
                .rounded(px(6.0))
                .bg(palette.subtle_fill)
                .border_1()
                .border_color(palette.card_border)
                .font_family(mono_font.clone())
                .text_size(px(12.0))
                .text_color(palette.text_primary)
                .child(command_text.to_string()),
        )
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_size(px(11.0))
                        .text_color(palette.text_muted)
                        .child(server_note),
                )
                .child(
                    Button::new("mcp-copy-command")
                        .outline()
                        .small()
                        .icon(Icon::new(if state.copied_command_toast {
                            IconName::Check
                        } else {
                            IconName::Copy
                        }))
                        .label(if state.copied_command_toast {
                            "Copied"
                        } else {
                            "Copy command"
                        })
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.copy_mcp_command(cmd_for_copy.clone(), cx);
                        })),
                ),
        )
        .into_any_element()
}

fn render_activity_card(
    state: &McpSettingsState,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let has_rows = !state.activity.is_empty();

    v_flex()
        .p_5()
        .gap_3p5()
        .rounded(px(8.0))
        .bg(palette.card_bg)
        .border_1()
        .border_color(palette.card_border)
        .child(
            h_flex()
                .items_center()
                .justify_between()
                .gap_3()
                .child(
                    v_flex()
                        .gap_0p5()
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(palette.text_primary)
                                .child("Recent agent activity"),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(palette.text_muted)
                                .child("Every call is recorded here, including the ones Larik refused."),
                        ),
                )
                .child(
                    h_flex()
                        .items_center()
                        .gap_1p5()
                        .child(
                            Button::new("mcp-refresh-activity")
                                .ghost()
                                .small()
                                .icon(Icon::new(IconName::RotateCcw))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.refresh_mcp_settings(cx);
                                })),
                        )
                        .when(has_rows, |row| {
                            row.child(
                                Button::new("mcp-clear-activity")
                                    .outline()
                                    .small()
                                    .label("Clear")
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(p) = this.settings_page.as_mut() {
                                            p.agents.confirming_clear_activity = true;
                                        }
                                        cx.notify();
                                    })),
                            )
                        }),
                ),
        )
        .child(if state.activity.is_empty() {
            div()
                .py_2()
                .text_size(px(12.0))
                .text_color(palette.text_muted)
                .child("No agent has connected yet.")
                .into_any_element()
        } else {
            v_flex()
                .gap_2()
                .children(state.activity.iter().map(|entry| {
                    render_activity_row(entry, palette, cx)
                }))
                .into_any_element()
        })
        .into_any_element()
}

fn render_activity_row(
    entry: &McpLogRecord,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let title = match entry.space_name.as_deref().filter(|s| !s.is_empty()) {
        Some(name) => format!("{} · {name}", entry.tool),
        None => entry.tool.clone(),
    };

    let detail = if let Some(reason) = entry.reason.as_deref().filter(|s| !s.trim().is_empty()) {
        format!("{} UTC · {}", entry.recorded_at, reason)
    } else {
        let rows_word = if entry.rows_returned == 1 { "row" } else { "rows" };
        format!(
            "{} UTC · {} {} · {} ms",
            entry.recorded_at, entry.rows_returned, rows_word, entry.duration_ms
        )
    };

    let (status_label, status_color) = match entry.status.as_str() {
        mcp_log_status::DENIED => ("Refused", palette.warning),
        mcp_log_status::ERROR => ("Failed", palette.danger),
        _ => ("Delivered", palette.success),
    };

    let can_open = entry
        .space_id
        .as_deref()
        .is_some_and(|s| !s.trim().is_empty())
        && entry
            .statement
            .as_deref()
            .is_some_and(|s| !s.trim().is_empty())
        && entry.tool != "get_schema";

    let open_space_id = entry.space_id.clone();
    let open_stmt = entry.statement.clone();
    let row_id = entry.id;

    h_flex()
        .px_3()
        .py_2()
        .rounded(px(6.0))
        .bg(palette.subtle_fill)
        .items_center()
        .justify_between()
        .gap_3()
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap(px(2.0))
                .child(
                    div()
                        .text_size(px(12.5))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(palette.text_primary)
                        .truncate()
                        .child(title),
                )
                .child(
                    div()
                        .text_size(px(11.0))
                        .text_color(palette.text_muted)
                        .truncate()
                        .child(detail),
                ),
        )
        .child(
            h_flex()
                .flex_shrink_0()
                .items_center()
                .gap_2()
                .when(can_open, |row| {
                    let sid = open_space_id.clone().unwrap_or_default();
                    let stmt = open_stmt.clone().unwrap_or_default();
                    row.child(
                        Button::new(SharedString::from(format!("mcp-open-tab-{row_id}")))
                            .ghost()
                            .small()
                            .label("Open as tab")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let _ = this.open_external_query(
                                    sid.clone(),
                                    None,
                                    stmt.clone(),
                                    window,
                                    cx,
                                );
                            })),
                    )
                })
                .child(
                    div()
                        .px_2()
                        .py(px(2.0))
                        .rounded_full()
                        .bg(palette.card_bg)
                        .text_size(px(10.5))
                        .font_weight(gpui_kit::FontWeight::MEDIUM)
                        .text_color(status_color)
                        .child(status_label),
                ),
        )
        .into_any_element()
}

fn render_clear_activity_confirm(
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    div()
        .id("mcp-clear-confirm-backdrop")
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.58))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .on_mouse_down(
            MouseButton::Left,
            cx.listener(|this, _, _, cx| {
                if let Some(p) = this.settings_page.as_mut() {
                    p.agents.confirming_clear_activity = false;
                }
                cx.notify();
            }),
        )
        .child(
            v_flex()
                .id("mcp-clear-confirm-card")
                .w(px(420.0))
                .max_w_full()
                .p_5()
                .gap_3p5()
                .rounded(px(10.0))
                .bg(palette.modal_bg)
                .border_1()
                .border_color(palette.card_border)
                .shadow_2xl()
                .overflow_hidden()
                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                .child(
                    div()
                        .text_size(px(15.0))
                        .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                        .text_color(palette.text_primary)
                        .child("Clear agent activity?"),
                )
                .child(
                    div()
                        .text_size(px(12.5))
                        .text_color(palette.text_secondary)
                        .child(
                            "This removes the record of every agent call, including the ones Larik refused. It cannot be undone.",
                        ),
                )
                .child(
                    h_flex()
                        .justify_end()
                        .gap_2()
                        .pt_1()
                        .child(
                            Button::new("mcp-clear-cancel")
                                .outline()
                                .small()
                                .label("Cancel")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    if let Some(p) = this.settings_page.as_mut() {
                                        p.agents.confirming_clear_activity = false;
                                    }
                                    cx.notify();
                                })),
                        )
                        .child(
                            Button::new("mcp-clear-confirm")
                                .danger()
                                .small()
                                .label("Clear")
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.clear_mcp_activity(cx);
                                })),
                        ),
                ),
        )
        .into_any_element()
}

pub fn render_mcp_approval_modal(
    pending: &PendingMcpApproval,
    queued_count: usize,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let mono_font = cx.theme().mono_font_family.clone();
    let req = &pending.request;
    let remaining = pending.remaining_secs();
    let mins = remaining / 60;
    let secs = remaining % 60;
    let countdown = format!("Auto-denies in {mins}:{secs:02}");
    let title = format!("An agent wants to run this in {}", req.space_name);
    let _timeout_secs = mcp_pipe::APPROVAL_TIMEOUT.as_secs();

    div()
        .id("mcp-approval-backdrop")
        .absolute()
        .inset_0()
        .bg(hsla(0.0, 0.0, 0.0, 0.62))
        .occlude()
        .flex()
        .items_center()
        .justify_center()
        .p_6()
        .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
        .child(
            v_flex()
                .id("mcp-approval-card")
                .w(px(580.0))
                .max_w_full()
                .p_5()
                .gap_3()
                .rounded(px(10.0))
                .bg(palette.modal_bg)
                .border_1()
                .border_color(palette.card_border)
                .shadow_2xl()
                .overflow_hidden()
                .child(
                    h_flex()
                        .items_center()
                        .gap_2p5()
                        .child(
                            div()
                                .size(px(28.0))
                                .rounded(px(6.0))
                                .bg(hsla(14.0 / 360.0, 0.85, 0.62, 0.18))
                                .flex()
                                .items_center()
                                .justify_center()
                                .child(
                                    Icon::new(IconName::Shield)
                                        .size(px(15.0))
                                        .text_color(rgb(0xF27B5E)),
                                ),
                        )
                        .child(
                            div()
                                .flex_1()
                                .min_w_0()
                                .text_size(px(15.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .text_color(palette.text_primary)
                                .child(title),
                        )
                        .when(queued_count > 0, |row| {
                            row.child(
                                div()
                                    .px_2()
                                    .py(px(2.0))
                                    .rounded_full()
                                    .bg(palette.subtle_fill)
                                    .text_size(px(10.5))
                                    .text_color(palette.text_secondary)
                                    .child(format!("+{queued_count} waiting")),
                            )
                        }),
                )
                .when_some(req.description.clone(), |col, desc| {
                    col.child(
                        div()
                            .text_size(px(13.0))
                            .text_color(palette.text_secondary)
                            .child(desc),
                    )
                })
                .when_some(req.reference.clone(), |col, reference| {
                    col.child(
                        h_flex().child(
                            div()
                                .px_2()
                                .py(px(2.0))
                                .rounded_full()
                                .bg(palette.subtle_fill)
                                .text_size(px(11.0))
                                .text_color(palette.text_secondary)
                                .child(format!("Ref: {reference}")),
                        ),
                    )
                })
                .child(
                    div()
                        .id("mcp-approval-sql-scroll")
                        .w_full()
                        .max_h(px(300.0))
                        .overflow_y_scroll()
                        .p_3()
                        .rounded(px(6.0))
                        .bg(palette.subtle_fill)
                        .border_1()
                        .border_color(palette.card_border)
                        .font_family(mono_font)
                        .text_size(px(12.0))
                        .line_height(px(19.0))
                        .text_color(palette.text_primary)
                        .child(req.statement.clone()),
                )
                .child(
                    h_flex()
                        .items_center()
                        .justify_between()
                        .pt_1()
                        .gap_3()
                        .child(
                            div()
                                .text_size(px(11.0))
                                .text_color(palette.text_muted)
                                .child(countdown),
                        )
                        .child(
                            h_flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new("mcp-approval-deny")
                                        .outline()
                                        .small()
                                        .label("Deny")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.resolve_active_mcp_approval(false, cx);
                                        })),
                                )
                                .child(
                                    Button::new("mcp-approval-approve")
                                        .primary()
                                        .small()
                                        .label("Run it")
                                        .on_click(cx.listener(|this, _, _, cx| {
                                            this.resolve_active_mcp_approval(true, cx);
                                        })),
                                ),
                        ),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_all_four_agent_client_commands_without_tokens() {
        let bin = Path::new(r"C:\Program Files\Larik\larik-mcp.exe");
        let state = Path::new(r"C:\Users\dev\AppData\Local\Larik SQL Studio");

        let claude = format_agent_command(AgentClientKind::ClaudeCode, Some(bin), Some(state));
        assert!(claude.starts_with("claude mcp add larik -- "));
        assert!(claude.contains("larik-mcp.exe"));
        assert!(claude.contains("--state"));

        let cursor = format_agent_command(AgentClientKind::Cursor, Some(bin), None);
        assert!(cursor.contains("\"mcpServers\""));
        assert!(cursor.contains("\"larik\""));

        let vscode = format_agent_command(AgentClientKind::VsCode, Some(bin), None);
        assert!(vscode.starts_with("code --add-mcp '"));
        assert!(vscode.contains("\"type\":\"stdio\""));

        let codex = format_agent_command(AgentClientKind::Codex, None, None);
        assert_eq!(codex, "codex mcp add larik -- larik-mcp");
    }
}
