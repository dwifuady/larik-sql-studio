use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use gpui_kit::assets::IconName;
use gpui_kit::component::button::{Button, ButtonVariants as _};
use gpui_kit::component::{Icon, Sizable as _, h_flex, v_flex};
use gpui_kit::prelude::FluentBuilder as _;
use gpui_kit::{
    AnyElement, Context, Hsla, InteractiveElement as _, IntoElement, ParentElement as _,
    SharedString, StatefulInteractiveElement as _, Styled as _, div, px,
};
use larik_update::{
    CheckResult, CheckStatus, DownloadProgress, InstallKind, UpdateError, UpdateInfo,
    UpdateService, can_self_update,
};

use crate::studio::StudioView;
use crate::theme::StudioPalette;

const BANNER_WIDTH: f32 = 420.0;
const BANNER_INNER_WIDTH: f32 = BANNER_WIDTH - 28.0;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdatePhase {
    Idle,
    Checking,
    Available,
    Downloading,
    ReadyToInstall,
    Installing,
    UpToDate,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateTone {
    Info,
    Success,
    Error,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum UpdateCheckOutcome {
    Available,
    UpToDate,
    Failed,
    Unsupported,
}

pub(crate) struct UpdateState {
    service: UpdateService,
    pub(crate) phase: UpdatePhase,
    pub(crate) info: Option<UpdateInfo>,
    pub(crate) downloaded: Option<PathBuf>,
    pub(crate) dismissed: bool,
    pub(crate) message: String,
    pub(crate) progress: Option<DownloadProgress>,
    pub(crate) last_check: Option<UpdateCheckOutcome>,
    pub(crate) download_gen: u64,
    cancel: Arc<AtomicBool>,
}

impl UpdateState {
    pub(crate) fn new(updates_root: PathBuf) -> Self {
        Self {
            service: UpdateService::new(updates_root),
            phase: UpdatePhase::Idle,
            info: None,
            downloaded: None,
            dismissed: false,
            message: String::new(),
            progress: None,
            last_check: None,
            download_gen: 0,
            cancel: Arc::new(AtomicBool::new(false)),
        }
    }

    pub(crate) fn supported(&self) -> bool {
        can_self_update()
    }

    pub(crate) fn current_version(&self) -> &str {
        self.service.current_version()
    }

    pub(crate) fn install_kind(&self) -> InstallKind {
        self.service.install_kind()
    }

    pub(crate) fn busy(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Checking | UpdatePhase::Downloading | UpdatePhase::Installing
        )
    }

    pub(crate) fn banner_visible(&self) -> bool {
        !self.dismissed
            && matches!(
                self.phase,
                UpdatePhase::Available
                    | UpdatePhase::Downloading
                    | UpdatePhase::ReadyToInstall
                    | UpdatePhase::Installing
                    | UpdatePhase::Failed
            )
    }

    pub(crate) fn action_label(&self) -> &'static str {
        match self.phase {
            UpdatePhase::Available => "Download",
            UpdatePhase::ReadyToInstall => "Install",
            UpdatePhase::Failed if self.downloaded.is_some() => "Try again",
            UpdatePhase::Failed => "Download again",
            _ => "",
        }
    }

    pub(crate) fn can_act(&self) -> bool {
        matches!(
            self.phase,
            UpdatePhase::Available | UpdatePhase::ReadyToInstall | UpdatePhase::Failed
        ) && self.info.is_some()
    }

    pub(crate) fn tone(&self) -> UpdateTone {
        match self.phase {
            UpdatePhase::Failed => UpdateTone::Error,
            UpdatePhase::ReadyToInstall => UpdateTone::Success,
            _ => UpdateTone::Info,
        }
    }

    pub(crate) fn title(&self) -> String {
        let version = self
            .info
            .as_ref()
            .map(|info| info.version.clone())
            .unwrap_or_default();
        match self.phase {
            UpdatePhase::Available => format!("Larik {version} is available"),
            UpdatePhase::Downloading => format!("Downloading Larik {version}"),
            UpdatePhase::ReadyToInstall => format!("Larik {version} is ready to install"),
            UpdatePhase::Installing => format!("Installing Larik {version}"),
            UpdatePhase::Failed if self.downloaded.is_none() && self.info.is_none() => {
                "Couldn't check for updates".to_string()
            }
            UpdatePhase::Failed => "Couldn't update Larik".to_string(),
            _ => String::new(),
        }
    }

    pub(crate) fn detail(&self) -> String {
        match self.phase {
            UpdatePhase::Available => {
                let size = format_size(
                    self.info
                        .as_ref()
                        .map(|info| info.asset_size)
                        .unwrap_or_default(),
                );
                if size.is_empty() {
                    format!(
                        "You're on {}. Download the update now, then install when it's ready.",
                        self.current_version()
                    )
                } else {
                    format!(
                        "You're on {}. Download the update ({size}) now, then install when it's ready.",
                        self.current_version()
                    )
                }
            }
            UpdatePhase::Downloading => match self.progress.and_then(|progress| progress.percent())
            {
                Some(percent) => format!(
                    "Downloading… {percent:.0}% of {}",
                    format_size(self.progress.and_then(|p| p.total).unwrap_or_default())
                ),
                None => format!(
                    "Downloading… {}",
                    format_size(self.progress.map(|p| p.received).unwrap_or_default())
                ),
            },
            UpdatePhase::ReadyToInstall => {
                if self.install_kind() == InstallKind::Installer {
                    "Click Install. Larik closes, the installer finishes the update, and Larik reopens."
                        .to_string()
                } else {
                    "Click Install. Larik closes, updates this folder, and reopens.".to_string()
                }
            }
            UpdatePhase::Installing => "Larik is closing to finish the update.".to_string(),
            UpdatePhase::Failed => self.message.clone(),
            _ => String::new(),
        }
    }

    fn dismiss(&mut self) {
        self.dismissed = true;
        self.cancel.store(true, Ordering::Relaxed);
    }

    pub(crate) fn status_line(&self, auto_check: bool) -> String {
        if self.busy() {
            return match self.phase {
                UpdatePhase::Downloading => "Downloading the update…".to_string(),
                UpdatePhase::Installing => "Installing the update…".to_string(),
                _ => "Checking for updates…".to_string(),
            };
        }
        match self.last_check {
            Some(UpdateCheckOutcome::Available) => self.detail(),
            Some(UpdateCheckOutcome::UpToDate) => "You're running the latest version.".to_string(),
            Some(UpdateCheckOutcome::Failed) => {
                "Couldn't check for updates. Check your connection and try again.".to_string()
            }
            Some(UpdateCheckOutcome::Unsupported) => {
                "This build does not update itself.".to_string()
            }
            None if auto_check => "A check runs once when Larik starts.".to_string(),
            None => "Automatic checks are off. Use Check now when you want to look.".to_string(),
        }
    }
}

fn format_size(bytes: u64) -> String {
    if bytes == 0 {
        return String::new();
    }
    const KB: f64 = 1024.0;
    let value = bytes as f64;
    if value >= KB * KB * KB {
        format!("{:.1} GB", value / (KB * KB * KB))
    } else if value >= KB * KB {
        format!("{:.1} MB", value / (KB * KB))
    } else {
        format!("{:.0} KB", value / KB)
    }
}

fn tone_color(tone: UpdateTone, palette: StudioPalette) -> Hsla {
    match tone {
        UpdateTone::Info => palette.accent,
        UpdateTone::Success => palette.success,
        UpdateTone::Error => palette.danger,
    }
}

pub(crate) fn render_update_banner(
    state: &UpdateState,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let tone = state.tone();
    let accent = tone_color(tone, palette);
    let title = state.title();
    let detail = state.detail();
    let downloading = state.phase == UpdatePhase::Downloading;
    let installing = state.phase == UpdatePhase::Installing;
    let show_progress = downloading || installing;
    let percent = state.progress.and_then(|progress| progress.percent());
    let indeterminate = installing || percent.is_none();
    let fill_width = BANNER_INNER_WIDTH * (percent.unwrap_or(0.0) as f32 / 100.0);
    let action_label = state.action_label();
    let can_act = state.can_act();

    v_flex()
        .id("update-banner")
        .absolute()
        .right(px(20.0))
        .bottom(px(44.0))
        .w(px(BANNER_WIDTH))
        .p(px(14.0))
        .gap_2()
        .rounded(px(10.0))
        .bg(palette.modal_bg)
        .border_1()
        .border_color(palette.card_border)
        .shadow_2xl()
        .text_color(palette.text_primary)
        .on_mouse_down(gpui_kit::MouseButton::Left, |_, _, cx| {
            cx.stop_propagation();
        })
        .child(
            h_flex()
                .items_start()
                .gap_2p5()
                .child(
                    div().flex_shrink_0().mt(px(1.0)).child(
                        Icon::new(match tone {
                            UpdateTone::Error => IconName::CircleAlert,
                            UpdateTone::Success => IconName::CircleCheck,
                            UpdateTone::Info => IconName::CircleArrowDown,
                        })
                        .size(px(16.0))
                        .text_color(accent),
                    ),
                )
                .child(
                    v_flex()
                        .flex_1()
                        .min_w_0()
                        .gap_1()
                        .child(
                            div()
                                .text_size(px(13.0))
                                .font_weight(gpui_kit::FontWeight::SEMIBOLD)
                                .child(SharedString::from(title)),
                        )
                        .child(
                            div()
                                .text_size(px(11.5))
                                .text_color(palette.text_muted)
                                .child(SharedString::from(detail)),
                        ),
                )
                .when(!installing, |row| {
                    row.child(
                        div()
                            .id("update-banner-close")
                            .flex_shrink_0()
                            .size(px(22.0))
                            .rounded(px(5.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor_pointer()
                            .hover(|style| style.bg(palette.subtle_hover))
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_update_notice(cx);
                            }))
                            .child(
                                Icon::new(IconName::X)
                                    .size(px(13.0))
                                    .text_color(palette.text_muted),
                            ),
                    )
                }),
        )
        .when(show_progress, |el| {
            el.child(
                div()
                    .h(px(4.0))
                    .w_full()
                    .rounded_full()
                    .bg(accent.opacity(0.2))
                    .child(
                        div()
                            .h_full()
                            .rounded_full()
                            .bg(accent)
                            .when(indeterminate, |bar| bar.w(px(BANNER_INNER_WIDTH * 0.3)))
                            .when(!indeterminate, |bar| bar.w(px(fill_width.max(2.0)))),
                    ),
            )
        })
        .when(!show_progress, |el| {
            el.child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("update-banner-later")
                            .ghost()
                            .small()
                            .label("Later")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.dismiss_update_notice(cx);
                            })),
                    )
                    .when(can_act, |row| {
                        row.child(
                            Button::new("update-banner-action")
                                .primary()
                                .small()
                                .label(action_label)
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.act_on_update(cx);
                                })),
                        )
                    }),
            )
        })
        .into_any_element()
}

pub(crate) fn render_check_toggle(
    id: &'static str,
    checked: bool,
    palette: StudioPalette,
    cx: &mut Context<StudioView>,
) -> AnyElement {
    let coral = gpui_kit::rgb(0xF27B5E);
    h_flex()
        .id(SharedString::from(format!("update-toggle-{id}")))
        .items_center()
        .gap_2()
        .cursor_pointer()
        .on_click(cx.listener(move |this, _, _, cx| {
            this.set_check_for_updates(!checked, cx);
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
                .flex_shrink_0()
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
                .child(div().size(px(15.0)).rounded_full().bg(if checked {
                    gpui_kit::rgb(0x1C1412).into()
                } else {
                    palette.text_secondary
                })),
        )
        .into_any_element()
}

enum DownloadEvent {
    Progress(DownloadProgress),
    Finished(Result<PathBuf, String>),
}
impl StudioView {
    pub(crate) fn run_launch_update_check(&mut self, cx: &mut Context<Self>) {
        if !self.update.supported() || !self.state.check_for_updates || self.update.busy() {
            return;
        }
        self.start_update_check(false, cx);
    }

    pub(crate) fn check_for_updates_now(&mut self, cx: &mut Context<Self>) {
        if !self.update.supported() || self.update.busy() {
            return;
        }
        self.update.dismissed = false;
        self.start_update_check(true, cx);
    }

    fn start_update_check(&mut self, manual: bool, cx: &mut Context<Self>) {
        let service = self.update.service.clone();
        self.update.phase = UpdatePhase::Checking;
        self.update.cancel.store(false, Ordering::Relaxed);
        if manual {
            self.update.dismissed = false;
        }
        cx.notify();

        cx.spawn(async move |this, cx| {
            let result = cx
                .background_executor()
                .spawn(async move { service.check() })
                .await;
            let _ = this.update(cx, |this, cx| {
                this.apply_update_check(result, manual, cx);
            });
        })
        .detach();
    }

    fn apply_update_check(&mut self, result: CheckResult, manual: bool, cx: &mut Context<Self>) {
        match result.status {
            CheckStatus::Available => {
                if let Some(info) = result.update.clone() {
                    self.update.downloaded = self.update.service.find_downloaded(&info);
                    self.update.phase = if self.update.downloaded.is_some() {
                        UpdatePhase::ReadyToInstall
                    } else {
                        UpdatePhase::Available
                    };
                    self.update.info = Some(info);
                    self.update.dismissed = false;
                    self.update.last_check = Some(UpdateCheckOutcome::Available);
                }
            }
            CheckStatus::UpToDate => {
                self.update.phase = UpdatePhase::UpToDate;
                self.update.info = None;
                self.update.downloaded = None;
                self.update.last_check = Some(UpdateCheckOutcome::UpToDate);
            }
            CheckStatus::Failed => {
                self.update.last_check = Some(UpdateCheckOutcome::Failed);
                self.update.message = match result.error.as_deref() {
                    Some(error) if !error.is_empty() => error.to_string(),
                    _ => "Larik couldn't check for updates. Check your connection and try again."
                        .to_string(),
                };
                self.update.phase = if self.update.downloaded.is_some() {
                    UpdatePhase::ReadyToInstall
                } else if self.update.info.is_some() {
                    UpdatePhase::Available
                } else if manual {
                    self.update.dismissed = false;
                    UpdatePhase::Failed
                } else {
                    UpdatePhase::Idle
                };
            }
            CheckStatus::Unavailable => {
                self.update.phase = UpdatePhase::Idle;
                if manual {
                    self.update.last_check = Some(UpdateCheckOutcome::Unsupported);
                }
            }
        }
        cx.notify();
    }

    pub(crate) fn act_on_update(&mut self, cx: &mut Context<Self>) {
        if !self.update.can_act() {
            return;
        }
        let Some(info) = self.update.info.clone() else {
            return;
        };
        if self.update.downloaded.is_some() {
            self.install_downloaded_update(&info, cx);
        } else {
            self.download_update(&info, cx);
        }
    }

    fn download_update(&mut self, info: &UpdateInfo, cx: &mut Context<Self>) {
        let service = self.update.service.clone();
        let info = info.clone();
        let cancel = Arc::clone(&self.update.cancel);
        cancel.store(false, Ordering::Relaxed);
        self.update.download_gen = self.update.download_gen.wrapping_add(1);
        let generation = self.update.download_gen;
        self.update.phase = UpdatePhase::Downloading;
        self.update.progress = Some(DownloadProgress {
            received: 0,
            total: (info.asset_size > 0).then_some(info.asset_size),
        });
        self.update.message.clear();
        cx.notify();

        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<DownloadEvent>();
        cx.background_executor()
            .spawn(async move {
                let mut report = {
                    let tx = tx.clone();
                    move |progress: DownloadProgress| {
                        let _ = tx.send(DownloadEvent::Progress(progress));
                    }
                };
                let outcome = service
                    .download(&info, &mut report, &cancel)
                    .map_err(|error| error.to_string());
                let _ = tx.send(DownloadEvent::Finished(outcome));
            })
            .detach();

        cx.spawn(async move |this, cx| {
            while let Some(event) = rx.recv().await {
                let done = matches!(event, DownloadEvent::Finished(_));
                let keep_going = this
                    .update(cx, |this, cx| {
                        if this.update.download_gen != generation {
                            return;
                        }
                        this.handle_download_event(event, cx);
                        cx.notify();
                    })
                    .is_ok();
                if !keep_going || done {
                    break;
                }
            }
        })
        .detach();
    }

    fn handle_download_event(&mut self, event: DownloadEvent, _cx: &mut Context<Self>) {
        match event {
            DownloadEvent::Progress(progress) => {
                self.update.progress = Some(progress);
            }
            DownloadEvent::Finished(Ok(path)) => {
                self.update.downloaded = Some(path);
                self.update.phase = UpdatePhase::ReadyToInstall;
            }
            DownloadEvent::Finished(Err(message)) => {
                self.update.downloaded = None;
                self.update.phase = UpdatePhase::Failed;
                self.update.message = message;
                self.update.dismissed = false;
            }
        }
    }

    fn install_downloaded_update(&mut self, info: &UpdateInfo, cx: &mut Context<Self>) {
        let Some(path) = self.update.downloaded.clone() else {
            return;
        };
        let service = self.update.service.clone();
        let info = info.clone();
        self.update.phase = UpdatePhase::Installing;
        cx.notify();

        cx.spawn(async move |this, cx| {
            let outcome = cx
                .background_executor()
                .spawn(async move { service.install(&info, &path) })
                .await;
            match outcome {
                Ok(()) => {
                    let _ = this.update(cx, |this, _cx| {
                        this.flush_before_update_exit();
                    });
                    cx.update(|cx| cx.quit());
                }
                Err(error) => {
                    let redownload =
                        matches!(error, UpdateError::Integrity(_) | UpdateError::Io(_));
                    let _ = this.update(cx, |this, cx| {
                        this.update.phase = UpdatePhase::Failed;
                        this.update.message = error.to_string();
                        this.update.dismissed = false;
                        if redownload {
                            this.update.downloaded = None;
                        }
                        cx.notify();
                    });
                }
            }
        })
        .detach();
    }

    pub(crate) fn dismiss_update_notice(&mut self, cx: &mut Context<Self>) {
        self.update.dismiss();
        if self.update.phase == UpdatePhase::Downloading {
            self.update.download_gen = self.update.download_gen.wrapping_add(1);
            self.update.phase = UpdatePhase::Available;
            self.update.progress = None;
        }
        cx.notify();
    }

    pub(crate) fn set_check_for_updates(&mut self, enabled: bool, cx: &mut Context<Self>) {
        if self.state.check_for_updates == enabled {
            return;
        }
        self.state.check_for_updates = enabled;
        self.persist(cx);
        cx.notify();
    }
}
