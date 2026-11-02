//! GPUI panel and gpui-tray status item.
//!
//! A left click on the status item toggles this panel under the menu bar.
//! On macOS the click is observed directly, because an accessory app often
//! never receives the status button's action. Right-click keeps Refresh,
//! Settings, and Quit. The panel itself is Chinese.

use std::sync::{Arc, OnceLock};
use std::time::{Duration, Instant};

use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, point, px, rgb, size, App, Bounds, ClipboardItem, Context, FontWeight, Global, Image,
    ImageFormat, ObjectFit, Pixels, Point, QuitMode, Render, TitlebarOptions, Window, WindowBounds,
    WindowHandle, WindowKind, WindowOptions,
};
use gpui_tray::{Icon, Tray};
use tokenbar_core::{
    delete_secret, format_compact, format_percent, load_settings, placeholder_snapshot, reset_hint,
    save_secret, save_settings, secret_presence, DisplaySettings, ProviderId, ProviderSnapshot,
    SecretKind, SecretPresence, TitleMode, UsageSnapshot,
};
const LOGO_SVG: &[u8] = include_bytes!("../../../assets/logo.svg");

const BG: u32 = 0x161618;
const CARD: u32 = 0x222226;
const LINE: u32 = 0x34343a;
const TEXT: u32 = 0xf2f2f4;
const MUTED: u32 = 0x8e8e93;
const ACCENT: u32 = 0x0a84ff;
const TRACK: u32 = 0x2c2c31;

struct Shell {
    tray: Tray,
    window: Option<WindowHandle<Panel>>,
    shown_at: Option<Instant>,
    snapshot: UsageSnapshot,
    status: String,
    show_settings: bool,
    expanded: Option<ProviderId>,
    saved: SecretPresence,
    settings: DisplaySettings,
    brand_icon: Icon,
}

/// Cocoa frame of the menu-bar item. Only the macOS click monitor fills it.
#[cfg_attr(not(target_os = "macos"), allow(dead_code))]
#[derive(Clone, Copy, Debug)]
pub struct StatusAnchor {
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}

/// Status-item commands. On macOS these arrive from the click monitor.
#[cfg(target_os = "macos")]
#[derive(Debug)]
pub enum TrayCommand {
    Toggle(Option<StatusAnchor>),
    Refresh,
    Settings,
    Quit,
    Dismiss,
}

impl Global for Shell {}

struct Panel;

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (generated, status, show_settings, saved, providers, expanded, settings) = {
            let shell = cx.global::<Shell>();
            (
                shell.snapshot.generated_at.clone(),
                shell.status.clone(),
                shell.show_settings,
                shell.saved,
                {
                    let mut rows: Vec<_> = shell
                        .snapshot
                        .providers
                        .iter()
                        .filter(|provider| shell.settings.shows(provider.id))
                        .cloned()
                        .collect();
                    rows.sort_by_key(|provider| panel_rank(provider.id));
                    rows
                },
                shell.expanded,
                shell.settings.clone(),
            )
        };
        let subtitle = if status.is_empty() {
            format!("{} 更新", clock_label(&generated))
        } else {
            status
        };

        let pending = all_pending(&providers);
        let cards: Vec<_> = providers
            .iter()
            .filter(|provider| has_panel_data(provider))
            .cloned()
            .collect();
        let empty_line = if providers.is_empty() {
            "没有要显示的服务"
        } else if pending {
            "正在读取"
        } else {
            "暂无用量"
        };

        div()
            .id("tokenbar-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(header(&subtitle))
            .child(
                div()
                    .id("tokenbar-body")
                    .flex_grow_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(10.))
                    .px(px(14.))
                    .pb(px(10.))
                    .children(
                        cards.iter().map(|provider| {
                            provider_card(provider, expanded == Some(provider.id), cx)
                        }),
                    )
                    .when(cards.is_empty(), |parent| {
                        parent.child(
                            div()
                                .px(px(2.))
                                .py(px(8.))
                                .text_size(px(13.))
                                .text_color(rgb(MUTED))
                                .child(empty_line),
                        )
                    })
                    .when(show_settings, |parent| {
                        parent.child(settings_card(saved, &settings, cx))
                    }),
            )
            .child(footer(cx))
    }
}

fn header(subtitle: &str) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .gap(px(10.))
        .px(px(16.))
        .pt(px(14.))
        .pb(px(12.))
        .child(logo_mark())
        .child(
            div()
                .flex()
                .flex_col()
                .gap(px(1.))
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .child("TokenBar"),
                )
                .child(
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .child(subtitle.to_string()),
                ),
        )
}

fn logo_mark() -> impl IntoElement {
    img(panel_logo())
        .id("tokenbar-logo")
        .w(px(28.))
        .h(px(28.))
        .object_fit(ObjectFit::Contain)
        .with_fallback(|| {
            div()
                .size(px(28.))
                .rounded(px(7.))
                .bg(rgb(0x0b1b3a))
                .into_any_element()
        })
}

fn panel_logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_SVG.to_vec())))
        .clone()
}

fn has_panel_data(provider: &ProviderSnapshot) -> bool {
    provider.used_percent.is_some()
        || provider.tokens.is_some()
        || provider.balance.is_some()
        || provider.cost.is_some()
        || provider
            .windows
            .iter()
            .any(|window| window.used_percent.is_some())
}

fn all_pending(providers: &[ProviderSnapshot]) -> bool {
    !providers.is_empty()
        && providers
            .iter()
            .all(|provider| provider.source == "pending" && !has_panel_data(provider))
}

fn panel_rank(id: ProviderId) -> u8 {
    match id {
        ProviderId::Claude => 0,
        ProviderId::Openai => 1,
        ProviderId::Opencode => 2,
        ProviderId::Deepseek => 3,
        ProviderId::Cursor => 4,
    }
}

fn provider_card(
    provider: &ProviderSnapshot,
    expanded: bool,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    let id = provider.id;
    let windows: Vec<_> = provider
        .windows
        .iter()
        .filter(|window| window.used_percent.is_some())
        .take(4)
        .cloned()
        .collect();
    let models: Vec<_> = provider.models.iter().take(6).cloned().collect();
    let secondary = secondary_line(provider);
    let failure = plan_failure_line(provider);
    let balance = provider.balance.clone().filter(|_| windows.is_empty());
    div()
        .id(row_id(id))
        .cursor_pointer()
        .flex()
        .flex_col()
        .gap(px(12.))
        .rounded_lg()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(LINE))
        .px(px(14.))
        .py(px(12.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .flex()
                        .items_center()
                        .gap(px(10.))
                        .child(provider_mark(id))
                        .child(
                            div()
                                .text_size(px(15.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(provider_name(id)),
                        ),
                )
                .when_some(provider.plan.clone(), |row, plan| {
                    row.child(plan_badge(plan))
                })
                .when(provider.plan.is_none() && balance.is_some(), |row| {
                    row.child(
                        div()
                            .text_size(px(18.))
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(balance.clone().unwrap_or_default()),
                    )
                }),
        )
        .when(!windows.is_empty(), |card| {
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(12.))
                    .children(windows.iter().map(|window| {
                        gauge(
                            tokenbar_core::window_label(&window.name),
                            window.used_percent.unwrap_or(0.0),
                            window.resets_at.as_deref(),
                        )
                    })),
            )
        })
        .when(windows.is_empty() && balance.is_none(), |card| {
            card.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(MUTED))
                    .child("暂无计划窗口"),
            )
        })
        .when_some(secondary, |card, line| {
            card.child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(line))
        })
        .when_some(failure, |card, line| {
            card.child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xff9f0a))
                    .whitespace_normal()
                    .child(line),
            )
        })
        .when(expanded && !models.is_empty(), |card| {
            card.child(
                div()
                    .flex()
                    .flex_col()
                    .gap(px(2.))
                    .children(models.iter().map(|model| {
                        div()
                            .flex()
                            .justify_between()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .child(model.name.clone())
                            .child(model_side(model))
                    })),
            )
        })
        .on_click(cx.listener(move |_this, _event, _window, cx| toggle_expanded(id, cx)))
}

fn provider_name(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Openai => "Codex",
        other => other.label(),
    }
}

fn provider_mark(id: ProviderId) -> impl IntoElement {
    let (text, bg) = match id {
        ProviderId::Claude => ("C", 0xd97757),
        ProviderId::Openai => ("Cx", 0x10a37f),
        ProviderId::Cursor => ("Cu", 0x7c6cf6),
        ProviderId::Opencode => ("Oc", 0x7c5cff),
        ProviderId::Deepseek => ("Ds", 0x4d6bfe),
    };
    div()
        .w(px(28.))
        .h(px(28.))
        .rounded(px(8.))
        .flex()
        .items_center()
        .justify_center()
        .bg(rgb(bg))
        .text_size(px(if text.len() > 1 { 11. } else { 14. }))
        .font_weight(FontWeight::SEMIBOLD)
        .text_color(rgb(0xffffff))
        .child(text)
}

fn plan_badge(plan: String) -> impl IntoElement {
    div()
        .px(px(8.))
        .py(px(2.))
        .rounded_full()
        .bg(rgb(0x2a2a2e))
        .text_size(px(11.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(TEXT))
        .child(plan)
}

fn secondary_line(provider: &ProviderSnapshot) -> Option<String> {
    let mut bits = Vec::new();
    if provider.detail.contains("沿用上次") {
        bits.push("沿用上次".to_string());
    }
    if let Some(tokens) = provider.tokens {
        if tokens > 0 {
            bits.push(format!("本地用量 {}", format_compact(tokens)));
        }
    }
    if let Some(cost) = provider.cost {
        bits.push(format!("花费 ${cost:.2}"));
    }
    if provider
        .windows
        .iter()
        .any(|window| window.used_percent.is_some())
    {
        if let Some(balance) = &provider.balance {
            bits.push(format!("余额 {balance}"));
        }
    }
    if bits.is_empty() {
        None
    } else {
        Some(bits.join(" · "))
    }
}

fn plan_failure_line(provider: &ProviderSnapshot) -> Option<String> {
    if provider
        .windows
        .iter()
        .any(|window| window.used_percent.is_some())
    {
        return None;
    }
    let blob = format!(
        "{} {}",
        provider.error.as_deref().unwrap_or(""),
        provider.detail
    );
    let lower = blob.to_ascii_lowercase();
    if lower.contains("error sending request")
        || lower.contains("timed out")
        || lower.contains("connection")
        || lower.contains("dns")
        || lower.contains("network")
        || lower.contains("wham")
        || lower.contains("http ")
        || lower.contains("invalid json")
    {
        return Some("用量服务暂时连不上，点刷新重试".to_string());
    }
    if provider.error.is_some() && provider.windows.is_empty() {
        return Some("暂时读不到计划额度，点刷新重试".to_string());
    }
    None
}

fn gauge(label: impl Into<String>, used: f64, resets_at: Option<&str>) -> impl IntoElement {
    let remaining = (100.0 - used).clamp(0.0, 100.0);
    let reset = resets_at.and_then(|stamp| {
        let hint = reset_hint(stamp);
        if hint.is_empty() {
            None
        } else {
            Some(hint)
        }
    });
    let ink = if remaining < 15.0 {
        0xff453a
    } else if remaining < 40.0 {
        0xff9f0a
    } else {
        0x30d158
    };
    div()
        .flex()
        .flex_col()
        .gap(px(6.))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .child(
                    div()
                        .text_size(px(13.))
                        .text_color(rgb(TEXT))
                        .child(label.into()),
                )
                .child(
                    div()
                        .text_size(px(15.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(ink))
                        .child(format!("剩余 {}%", format_percent(remaining))),
                ),
        )
        .child(remaining_bar(remaining))
        .when_some(reset, |row, hint| {
            row.child(div().text_size(px(12.)).text_color(rgb(MUTED)).child(hint))
        })
}

fn remaining_bar(remaining: f64) -> impl IntoElement {
    let pct = remaining.clamp(0.0, 100.0) as f32;
    let color = if remaining < 15.0 {
        0xff453a
    } else if remaining < 40.0 {
        0xff9f0a
    } else {
        0x30d158
    };
    div()
        .flex()
        .h(px(8.))
        .w_full()
        .rounded_full()
        .bg(rgb(0x3a3a3c))
        .child(
            div()
                .h(px(8.))
                .flex_grow(pct.max(0.01))
                .rounded_full()
                .bg(rgb(color)),
        )
        .child(div().h(px(8.)).flex_grow((100.0 - pct).max(0.01)))
}

fn model_side(model: &tokenbar_core::ModelUsage) -> String {
    match model.cost {
        Some(cost) => format!("{}  ${cost:.2}", format_compact(model.tokens)),
        None => format_compact(model.tokens),
    }
}

fn footer(cx: &mut Context<Panel>) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(12.))
        .py(px(10.))
        .border_t_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(text_button("refresh", "刷新", true, cx, |_, _, cx| {
                    refresh_now(cx);
                }))
                .child(text_button("settings", "设置", false, cx, |_, _, cx| {
                    let open = !cx.global::<Shell>().show_settings;
                    cx.global_mut::<Shell>().show_settings = open;
                    cx.notify();
                })),
        )
        .child(text_button("quit", "退出", false, cx, |_, _, cx| {
            quit(cx)
        }))
}

fn text_button<F>(
    id: &'static str,
    label: &'static str,
    accent: bool,
    cx: &mut Context<Panel>,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static,
{
    let fill = if accent { ACCENT } else { TRACK };
    let ink = if accent { 0xffffff } else { TEXT };
    div()
        .id(id)
        .cursor_pointer()
        .px(px(12.))
        .py(px(5.))
        .rounded_md()
        .bg(rgb(fill))
        .text_size(px(12.))
        .font_weight(FontWeight::MEDIUM)
        .text_color(rgb(ink))
        .child(label)
        .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
}

fn settings_card(
    saved: SecretPresence,
    settings: &DisplaySettings,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .px(px(12.))
        .py(px(10.))
        .rounded_lg()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .text_size(px(12.))
                .font_weight(FontWeight::SEMIBOLD)
                .child("设置"),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("显示"),
        )
        .child(toggle_row(
            "show-cursor",
            "Cursor",
            settings.show_cursor,
            cx,
            |settings| {
                settings.show_cursor = !settings.show_cursor;
            },
        ))
        .child(toggle_row(
            "show-claude",
            "Claude",
            settings.show_claude,
            cx,
            |settings| {
                settings.show_claude = !settings.show_claude;
            },
        ))
        .child(toggle_row(
            "show-openai",
            "OpenAI",
            settings.show_openai,
            cx,
            |settings| {
                settings.show_openai = !settings.show_openai;
            },
        ))
        .child(toggle_row(
            "show-opencode",
            "OpenCode",
            settings.show_opencode,
            cx,
            |settings| {
                settings.show_opencode = !settings.show_opencode;
            },
        ))
        .child(toggle_row(
            "show-deepseek",
            "DeepSeek",
            settings.show_deepseek,
            cx,
            |settings| {
                settings.show_deepseek = !settings.show_deepseek;
            },
        ))
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("托盘优先"),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .flex_wrap()
                .child(primary_button(
                    "primary-auto",
                    "自动",
                    settings.primary.is_none(),
                    None,
                    cx,
                ))
                .child(primary_button(
                    "primary-claude",
                    "Claude",
                    settings.primary == Some(ProviderId::Claude),
                    Some(ProviderId::Claude),
                    cx,
                ))
                .child(primary_button(
                    "primary-openai",
                    "OpenAI",
                    settings.primary == Some(ProviderId::Openai),
                    Some(ProviderId::Openai),
                    cx,
                ))
                .child(primary_button(
                    "primary-cursor",
                    "Cursor",
                    settings.primary == Some(ProviderId::Cursor),
                    Some(ProviderId::Cursor),
                    cx,
                )),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("菜单栏"),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(mode_button(
                    "mode-bars",
                    "条状余额",
                    settings.title_mode == TitleMode::BalanceBars,
                    TitleMode::BalanceBars,
                    cx,
                ))
                .child(mode_button(
                    "mode-text",
                    "百分比",
                    settings.title_mode == TitleMode::PercentText,
                    TitleMode::PercentText,
                    cx,
                ))
                .child(mode_button(
                    "mode-icon",
                    "仅图标",
                    settings.title_mode == TitleMode::IconOnly,
                    TitleMode::IconOnly,
                    cx,
                )),
        )
        .child(
            div()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("刷新"),
        )
        .child(
            div()
                .flex()
                .gap(px(6.))
                .child(interval_button(
                    "every-60",
                    "1 分钟",
                    settings.refresh_secs == 60,
                    60,
                    cx,
                ))
                .child(interval_button(
                    "every-300",
                    "5 分钟",
                    settings.refresh_secs == 300,
                    300,
                    cx,
                ))
                .child(interval_button(
                    "every-900",
                    "15 分钟",
                    settings.refresh_secs == 900,
                    900,
                    cx,
                )),
        )
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(rgb(MUTED))
                .whitespace_normal()
                .child("凭据只保存在本机。"),
        )
        .child(secret_row(
            "paste-cursor",
            "clear-cursor",
            "粘贴 Cursor 会话",
            saved.cursor,
            SecretKind::CursorSession,
            cx,
        ))
        .child(secret_row(
            "paste-claude",
            "clear-claude",
            "粘贴 Claude 凭据",
            saved.claude,
            SecretKind::ClaudeOauth,
            cx,
        ))
        .child(secret_row(
            "paste-openai",
            "clear-openai",
            "粘贴 OpenAI 密钥",
            saved.openai,
            SecretKind::OpenaiKey,
            cx,
        ))
        .child(secret_row(
            "paste-deepseek",
            "clear-deepseek",
            "粘贴 DeepSeek 密钥",
            saved.deepseek,
            SecretKind::DeepseekKey,
            cx,
        ))
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(rgb(MUTED))
                .whitespace_normal()
                .child("未粘贴时，自动读取本机登录。"),
        )
}

fn secret_row(
    id: &'static str,
    clear_id: &'static str,
    label: &'static str,
    saved: bool,
    kind: SecretKind,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .gap(px(8.))
        .child(
            div()
                .id(id)
                .cursor_pointer()
                .px(px(10.))
                .py(px(5.))
                .rounded_md()
                .bg(rgb(TRACK))
                .text_size(px(12.))
                .child(label)
                .on_click(cx.listener(move |_this, _event, _window, cx| paste(kind, cx))),
        )
        .child(
            div()
                .flex_grow_1()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child(if saved { "已保存" } else { "未设置" }),
        )
        .child(
            div()
                .id(clear_id)
                .cursor_pointer()
                .text_size(px(11.))
                .text_color(rgb(MUTED))
                .child("清除")
                .on_click(cx.listener(move |_this, _event, _window, cx| clear_secret(kind, cx))),
        )
}

fn toggle_row(
    id: &'static str,
    label: &'static str,
    on: bool,
    cx: &mut Context<Panel>,
    mutate: impl Fn(&mut DisplaySettings) + 'static,
) -> impl IntoElement {
    div()
        .id(id)
        .cursor_pointer()
        .flex()
        .items_center()
        .justify_between()
        .px(px(10.))
        .py(px(5.))
        .rounded_md()
        .bg(rgb(TRACK))
        .child(div().text_size(px(12.)).child(label))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(if on { ACCENT } else { MUTED }))
                .child(if on { "显示" } else { "隐藏" }),
        )
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            commit_settings(cx, &mutate);
        }))
}

fn primary_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    primary: Option<ProviderId>,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    choice(id, label, selected, cx, move |settings| {
        settings.primary = primary;
    })
}

fn mode_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    mode: TitleMode,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    choice(id, label, selected, cx, move |settings| {
        settings.title_mode = mode;
    })
}

fn interval_button(
    id: &'static str,
    label: &'static str,
    selected: bool,
    secs: u64,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    choice(id, label, selected, cx, move |settings| {
        settings.refresh_secs = secs;
    })
}

fn choice<F>(
    id: &'static str,
    label: &'static str,
    selected: bool,
    cx: &mut Context<Panel>,
    mutate: F,
) -> impl IntoElement
where
    F: Fn(&mut DisplaySettings) + 'static,
{
    div()
        .id(id)
        .cursor_pointer()
        .px(px(10.))
        .py(px(5.))
        .rounded_md()
        .bg(rgb(if selected { ACCENT } else { TRACK }))
        .text_size(px(12.))
        .text_color(rgb(if selected { 0xffffff } else { TEXT }))
        .child(label)
        .on_click(cx.listener(move |_this, _event, _window, cx| {
            commit_settings(cx, &mutate);
        }))
}

fn commit_settings(cx: &mut Context<Panel>, mutate: &impl Fn(&mut DisplaySettings)) {
    let mut settings = cx.global::<Shell>().settings.clone();
    mutate(&mut settings);
    let _ = save_settings(&settings);
    cx.global_mut::<Shell>().settings = settings;
    publish_title(cx);
    cx.notify();
}

fn publish_title(cx: &mut App) {
    let (mode, title, tooltip, brand, providers) = {
        let shell = cx.global::<Shell>();
        let visible: Vec<ProviderSnapshot> = shell
            .snapshot
            .providers
            .iter()
            .filter(|provider| shell.settings.shows(provider.id))
            .cloned()
            .collect();
        let title = shell.settings.menu_title(&shell.snapshot.providers);
        let tooltip = tokenbar_core::status_item_title(&visible, TitleMode::PercentText);
        (
            shell.settings.title_mode,
            title,
            tooltip,
            shell.brand_icon.clone(),
            visible,
        )
    };
    let tray = cx.global::<Shell>().tray.clone();
    let _ = tray.set_tooltip(Some(format!("TokenBar {tooltip}")), cx);
    if mode == TitleMode::IconOnly {
        let _ = tray.set_title(None::<String>, cx);
        match meter_icon(&providers) {
            Ok(icon) => {
                let _ = tray.set_icon(icon, cx);
            }
            Err(_) => {
                let _ = tray.set_icon(brand, cx);
            }
        }
        return;
    }
    let shown = if title.is_empty() { None } else { Some(title) };
    let _ = tray.set_title(shown, cx);
    let _ = tray.set_icon(brand, cx);
}

fn clock_label(generated_at: &str) -> String {
    if generated_at.len() >= 16 && generated_at.as_bytes().get(10) == Some(&b'T') {
        generated_at[11..16].to_string()
    } else if generated_at.is_empty() {
        "—".to_string()
    } else {
        generated_at.to_string()
    }
}

fn row_id(id: ProviderId) -> &'static str {
    match id {
        ProviderId::Cursor => "row-cursor",
        ProviderId::Claude => "row-claude",
        ProviderId::Openai => "row-openai",
        ProviderId::Opencode => "row-opencode",
        ProviderId::Deepseek => "row-deepseek",
    }
}

fn kind_name(kind: SecretKind) -> &'static str {
    match kind {
        SecretKind::CursorSession => "Cursor 会话",
        SecretKind::ClaudeOauth => "Claude 凭据",
        SecretKind::OpenaiKey => "OpenAI 密钥",
        SecretKind::DeepseekKey => "DeepSeek 密钥",
    }
}

fn toggle_expanded(id: ProviderId, cx: &mut Context<Panel>) {
    let current = cx.global::<Shell>().expanded;
    cx.global_mut::<Shell>().expanded = if current == Some(id) { None } else { Some(id) };
    cx.notify();
}

fn paste(kind: SecretKind, cx: &mut Context<Panel>) {
    let Some(item) = cx.read_from_clipboard() else {
        set_status(cx, "剪贴板是空的");
        return;
    };
    let Some(text) = clipboard_text(&item) else {
        set_status(cx, "剪贴板没有文本");
        return;
    };
    match save_secret(kind, &text) {
        Ok(()) => {
            refresh_saved(cx);
            set_status(cx, &format!("{}已保存", kind_name(kind)));
            refresh_now(cx);
        }
        Err(err) => set_status(cx, &err),
    }
}

fn clear_secret(kind: SecretKind, cx: &mut Context<Panel>) {
    match delete_secret(kind) {
        Ok(()) => {
            refresh_saved(cx);
            set_status(cx, &format!("已清除{}", kind_name(kind)));
        }
        Err(err) => set_status(cx, &err),
    }
}

fn clipboard_text(item: &ClipboardItem) -> Option<String> {
    let text = item.text()?.trim().to_string();
    if text.is_empty() {
        None
    } else {
        Some(text)
    }
}

fn refresh_saved(cx: &mut Context<Panel>) {
    cx.global_mut::<Shell>().saved = secret_presence();
    cx.notify();
}

fn set_status(cx: &mut Context<Panel>, status: &str) {
    cx.global_mut::<Shell>().status = status.to_string();
    cx.notify();
}

fn refresh_now(cx: &mut App) {
    cx.global_mut::<Shell>().status = "正在刷新…".to_string();
    cx.spawn(async move |cx| {
        let snapshot = cx
            .background_spawn(async { tokenbar_core::collect_snapshot() })
            .await;
        cx.update(|cx| apply_snapshot(cx, snapshot));
    })
    .detach();
}

fn keep_last_known(previous: &[ProviderSnapshot], fresh: &mut [ProviderSnapshot]) {
    for provider in fresh {
        if provider.used_percent.is_some() || provider.tokens.is_some() {
            continue;
        }
        let Some(old) = previous.iter().find(|item| item.id == provider.id) else {
            continue;
        };
        if old.used_percent.is_none() && old.tokens.is_none() {
            continue;
        }
        let error = provider.error.clone();
        let mut kept = old.clone();
        kept.error = error.or(kept.error);
        if !kept.detail.contains("沿用上次") {
            kept.detail = if kept.detail.is_empty() {
                "沿用上次".to_string()
            } else {
                format!("沿用上次 · {}", kept.detail)
            };
        }
        *provider = kept;
    }
}

fn apply_snapshot(cx: &mut App, mut snapshot: UsageSnapshot) {
    let previous = cx.global::<Shell>().snapshot.providers.clone();
    keep_last_known(&previous, &mut snapshot.providers);
    {
        let shell = cx.global_mut::<Shell>();
        shell.snapshot = snapshot;
        if shell.status == "正在刷新…" || shell.status == "正在读取" {
            shell.status.clear();
        }
    }
    publish_title(cx);
    let window = cx.global::<Shell>().window;
    if let Some(window) = window {
        let alive = window.update(cx, |_panel, _window, cx| cx.notify()).is_ok();
        if !alive {
            cx.global_mut::<Shell>().window = None;
        }
    }
}

fn quit(cx: &mut App) {
    let tray = cx.try_global::<Shell>().map(|shell| shell.tray.clone());
    if let Some(tray) = tray {
        let _ = tray.close(cx);
    }
    cx.quit();
}

pub fn run() {
    gpui_kit::application()
        .with_quit_mode(QuitMode::Explicit)
        .run(|cx| {
            gpui_kit::init(cx);
            if let Err(err) = setup(cx) {
                eprintln!("TokenBar failed to start: {err}");
                cx.quit();
            }
        });
}

fn setup(cx: &mut App) -> gpui_tray::Result<()> {
    set_accessory();
    let brand = tray_icon(cx)?;
    let tray = Tray::builder()
        .icon(brand.clone())
        .title("TokenBar")
        .tooltip("TokenBar")
        .on_activate_with(|_token, cx| toggle_panel(cx, None))
        .build(cx)?;
    cx.set_global(Shell {
        tray,
        window: None,
        shown_at: None,
        snapshot: placeholder_snapshot(),
        status: "正在读取".to_string(),
        show_settings: false,
        expanded: None,
        saved: secret_presence(),
        settings: load_settings(),
        brand_icon: brand,
    });
    start_refresh_loop(cx);
    publish_title(cx);
    install_click_bridge(cx);
    eprintln!("TokenBar is running in the menu bar. Click the icon to open the panel.");
    Ok(())
}

fn install_click_bridge(cx: &mut App) {
    #[cfg(target_os = "macos")]
    {
        let (tx, rx) = async_channel::unbounded();
        crate::status_click::install(tx);
        cx.spawn(async move |cx| {
            while let Ok(command) = rx.recv().await {
                cx.update(|cx| dispatch_tray_command(command, cx));
            }
        })
        .detach();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = cx;
    }
}

#[cfg(target_os = "macos")]
fn dispatch_tray_command(command: TrayCommand, cx: &mut App) {
    match command {
        TrayCommand::Toggle(anchor) => toggle_panel(cx, anchor),
        TrayCommand::Refresh => refresh_now(cx),
        TrayCommand::Settings => {
            if !panel_is_open(cx) {
                show_panel(cx, None);
            }
            cx.global_mut::<Shell>().show_settings = true;
            notify_panel(cx);
        }
        TrayCommand::Quit => quit(cx),
        TrayCommand::Dismiss => {
            let recent = cx
                .global::<Shell>()
                .shown_at
                .is_some_and(|opened| opened.elapsed() < Duration::from_millis(250));
            if !recent {
                hide_panel(cx);
            }
        }
    }
}

#[cfg(target_os = "macos")]
fn notify_panel(cx: &mut App) {
    let window = cx.global::<Shell>().window;
    if let Some(window) = window {
        let _ = window.update(cx, |_panel, _window, cx| cx.notify());
    }
}

/// macOS menu-bar apps use the accessory activation policy so they can open
/// windows without taking a Dock slot. gpui-kit 0.6 does not expose that
/// switch, so this calls AppKit directly. `LSUIElement` in the packaged
/// Info.plist covers launches of the `.app` bundle as well.
#[cfg(target_os = "macos")]
fn set_activation_policy(accessory: bool) {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(marker);
    let policy = if accessory {
        NSApplicationActivationPolicy::Accessory
    } else {
        NSApplicationActivationPolicy::Regular
    };
    let _ = app.setActivationPolicy(policy);
}

#[cfg(not(target_os = "macos"))]
fn set_activation_policy(_accessory: bool) {}

fn set_accessory() {
    set_activation_policy(true);
}

const PANEL_WIDTH: f32 = 392.0;
const PANEL_HEIGHT: f32 = 520.0;

fn panel_is_open(cx: &App) -> bool {
    cx.global::<Shell>().window.is_some()
}

fn toggle_panel(cx: &mut App, anchor: Option<StatusAnchor>) {
    // The status-button action and the click monitor can both see one click.
    if !accept_toggle() {
        return;
    }
    if panel_is_open(cx) {
        hide_panel(cx);
    } else {
        show_panel(cx, anchor);
    }
}

fn accept_toggle() -> bool {
    use std::sync::Mutex;
    static LAST: Mutex<Option<Instant>> = Mutex::new(None);
    let Ok(mut last) = LAST.lock() else {
        return true;
    };
    let now = Instant::now();
    if last.is_some_and(|opened| now.duration_since(opened) < Duration::from_millis(180)) {
        return false;
    }
    *last = Some(now);
    true
}

fn hide_panel(cx: &mut App) {
    let window = cx.global::<Shell>().window;
    if let Some(window) = window {
        let _ = window.update(cx, |_panel, window, _cx| window.remove_window());
    }
    {
        let shell = cx.global_mut::<Shell>();
        shell.window = None;
        shell.shown_at = None;
    }
    #[cfg(target_os = "macos")]
    crate::status_click::set_panel_open(false);
    set_activation_policy(true);
}

fn show_panel(cx: &mut App, anchor: Option<StatusAnchor>) {
    // Accessory policy keeps the Dock icon hidden, and it also keeps a newly
    // opened window from coming forward. Match the menu-bar pattern: become a
    // regular app for the time the panel is open, then drop back.
    set_activation_policy(false);
    cx.activate(true);
    let origin = panel_origin(cx, anchor);
    match cx.open_window(panel_options(origin), |_window, cx| cx.new(|_| Panel)) {
        Ok(handle) => {
            cx.global_mut::<Shell>().window = Some(handle);
            cx.global_mut::<Shell>().shown_at = Some(Instant::now());
            #[cfg(target_os = "macos")]
            crate::status_click::set_panel_open(true);
            let _ = handle.update(cx, |_panel, window, _cx| window.activate_window());
        }
        Err(err) => {
            eprintln!("failed to open TokenBar panel: {err}");
            cx.global_mut::<Shell>().status = "无法打开面板".to_string();
            #[cfg(target_os = "macos")]
            crate::status_click::set_panel_open(false);
            set_activation_policy(true);
        }
    }
}

fn panel_origin(cx: &App, anchor: Option<StatusAnchor>) -> Point<Pixels> {
    #[cfg(target_os = "macos")]
    if let Some(anchor) = anchor {
        if let Some((x, y)) = crate::status_click::origin_under(&anchor, PANEL_WIDTH) {
            return point(px(x), px(y));
        }
    }
    #[cfg(not(target_os = "macos"))]
    let _ = anchor;
    let width = cx
        .primary_display()
        .map(|display| display.bounds().size.width)
        .unwrap_or(px(1280.));
    point(width - px(PANEL_WIDTH) - px(12.), px(8.))
}

fn panel_options(origin: Point<Pixels>) -> WindowOptions {
    WindowOptions {
        focus: true,
        show: true,
        is_resizable: false,
        is_movable: true,
        is_minimizable: false,
        kind: WindowKind::Floating,
        window_min_size: Some(size(px(360.), px(280.))),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin,
            size: size(px(PANEL_WIDTH), px(PANEL_HEIGHT)),
        })),
        titlebar: Some(TitlebarOptions {
            title: None,
            appears_transparent: true,
            traffic_light_position: Some(point(px(-80.), px(-80.))),
        }),
        ..Default::default()
    }
}

fn start_refresh_loop(cx: &mut App) {
    cx.spawn(async move |cx| loop {
        let snapshot = cx
            .background_spawn(async { tokenbar_core::collect_snapshot() })
            .await;
        cx.update(|cx| apply_snapshot(cx, snapshot));
        let wait = cx.update(|cx| Duration::from_secs(cx.global::<Shell>().settings.refresh_secs));
        cx.background_executor().timer(wait).await;
    })
    .detach();
}

fn tray_icon(_cx: &App) -> gpui_tray::Result<Icon> {
    // A template silhouette, not the full-color app mark. The color logo
    // turns into a muddy blue ring at menu-bar size.
    let builder = Icon::from_rgba(template_mark_pixels(), 32, 32);
    #[cfg(target_os = "macos")]
    let builder = builder.template(true).scale_factor(2.0);
    builder.build()
}

fn template_mark_pixels() -> Vec<u8> {
    const N: u32 = 32;
    let mut rgba = vec![0_u8; (N * N * 4) as usize];
    for y in 0..N {
        for x in 0..N {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            let distance = (px - 15.5).hypot(py - 15.5);
            let ring = (1.6 - (distance - 11.2).abs()).clamp(0.0, 1.0);
            let bar = rect_coverage(px, 3.0, 29.0) * rect_coverage(py, 13.2, 18.8);
            let alpha = ring.max(bar);
            if alpha <= 0.0 {
                continue;
            }
            let offset = ((y * N + x) * 4) as usize;
            rgba[offset + 3] = (alpha * 255.0).round() as u8;
        }
    }
    rgba
}

fn rect_coverage(value: f32, start: f32, end: f32) -> f32 {
    let enter = (value - start + 0.5).clamp(0.0, 1.0);
    let leave = (end - value + 0.5).clamp(0.0, 1.0);
    enter.min(leave)
}

fn meter_icon(providers: &[ProviderSnapshot]) -> gpui_tray::Result<Icon> {
    let rows: Vec<f64> = providers
        .iter()
        .filter_map(|provider| provider.used_percent.map(tokenbar_core::remaining_percent))
        .take(3)
        .collect();
    if rows.is_empty() {
        return Err(gpui_tray::Error::InvalidIcon("no quota to draw".into()));
    }
    const SIZE: u32 = 36;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    let count = rows.len() as u32;
    let bar_h = 5u32;
    let gap = 3u32;
    let total_h = count * bar_h + (count - 1) * gap;
    let top = (SIZE - total_h) / 2;
    let inset = 4u32;
    let inner = SIZE - inset * 2;
    for (index, remaining) in rows.iter().enumerate() {
        let y0 = top + index as u32 * (bar_h + gap);
        let color = remaining_rgb(*remaining);
        let fill = ((inner as f64) * (remaining / 100.0)).round() as u32;
        for y in y0..y0 + bar_h {
            for x in inset..SIZE - inset {
                let painted = if x < inset + fill {
                    [color[0], color[1], color[2], 255]
                } else {
                    [0x8E, 0x8E, 0x93, 150]
                };
                let offset = ((y * SIZE + x) * 4) as usize;
                rgba[offset..offset + 4].copy_from_slice(&painted);
            }
        }
    }
    let builder = Icon::from_rgba(rgba, SIZE, SIZE);
    #[cfg(target_os = "macos")]
    let builder = builder.template(false).scale_factor(2.0);
    builder.build()
}

fn remaining_rgb(remaining: f64) -> [u8; 3] {
    if remaining >= 50.0 {
        [0x30, 0xD1, 0x58]
    } else if remaining >= 20.0 {
        [0xFF, 0x9F, 0x0A]
    } else {
        [0xFF, 0x45, 0x3A]
    }
}
