//! GPUI panel and gpui-tray status item.
//!
//! Primary click opens the panel. The panel is the place for refresh,
//! settings, and quit. There is no native menu, because on macOS a menu
//! would consume the click that should open the panel.
//!
//! The panel speaks Chinese and names providers in full. The tray title
//! stays the compact `R42% C72% O1.2k` string. Paths and request URLs stay
//! in `--check` JSON; the panel only shows them after a row is expanded.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    div, img, point, px, rgb, size, App, Bounds, ClipboardItem, Context, FontWeight, Global, Image,
    ImageFormat, ObjectFit, QuitMode, Render, TitlebarOptions, Window, WindowBounds, WindowHandle,
    WindowOptions,
};
use gpui_tray::{Icon, Tray};
use tokenbar_core::{
    delete_secret, format_compact, format_percent, load_settings, placeholder_snapshot, reset_hint,
    save_secret, save_settings, secret_presence, DisplaySettings, ProviderId, ProviderSnapshot,
    SecretKind, SecretPresence, TitleMode, UsageSnapshot,
};
const LOGO_SVG: &[u8] = include_bytes!("../../../assets/logo.svg");
const TRAY_SVG: &[u8] = include_bytes!("../../../assets/logo-16.svg");

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
    snapshot: UsageSnapshot,
    status: String,
    show_settings: bool,
    expanded: Option<ProviderId>,
    saved: SecretPresence,
    settings: DisplaySettings,
}

impl Global for Shell {}

struct Panel;

struct RowFace {
    value: String,
    subtitle: String,
    technical: String,
    quiet_value: bool,
    bar: Option<f64>,
}

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (generated, status, show_settings, saved, providers, expanded, settings) = {
            let shell = cx.global::<Shell>();
            (
                shell.snapshot.generated_at.clone(),
                shell.status.clone(),
                shell.show_settings,
                shell.saved,
                shell
                    .snapshot
                    .providers
                    .iter()
                    .filter(|provider| shell.settings.shows(provider.id))
                    .cloned()
                    .collect::<Vec<_>>(),
                shell.expanded,
                shell.settings.clone(),
            )
        };
        let subtitle = if status.is_empty() {
            format!("{} 更新", clock_label(&generated))
        } else {
            status
        };

        div()
            .id("tokenbar-root")
            .size_full()
            .flex()
            .flex_col()
            .bg(rgb(BG))
            .text_color(rgb(TEXT))
            .child(header(&subtitle, &providers, settings.title_mode))
            .child(
                div()
                    .id("tokenbar-body")
                    .flex_grow_1()
                    .overflow_y_scroll()
                    .flex()
                    .flex_col()
                    .gap(px(8.))
                    .px(px(12.))
                    .pb(px(8.))
                    .children(
                        providers.iter().map(|provider| {
                            provider_card(provider, expanded == Some(provider.id), cx)
                        }),
                    )
                    .when(providers.is_empty(), |parent| {
                        parent.child(
                            div()
                                .px(px(4.))
                                .text_size(px(12.))
                                .text_color(rgb(MUTED))
                                .child("在设置里打开要看的服务"),
                        )
                    })
                    .when(show_settings, |parent| {
                        parent.child(settings_card(saved, &settings, cx))
                    }),
            )
            .child(footer(cx))
    }
}

fn header(subtitle: &str, providers: &[ProviderSnapshot], mode: TitleMode) -> impl IntoElement {
    div()
        .flex()
        .items_center()
        .justify_between()
        .px(px(14.))
        .pt(px(12.))
        .pb(px(10.))
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .child(logo_mark())
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(1.))
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child("TokenBar"),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .text_color(rgb(MUTED))
                                .child(subtitle.to_string()),
                        ),
                ),
        )
        .child(
            div()
                .flex()
                .items_center()
                .gap(px(8.))
                .children(providers.iter().filter_map(|provider| {
                    let label = provider.menu_label(mode)?;
                    Some(
                        div()
                            .text_size(px(11.))
                            .text_color(rgb(MUTED))
                            .whitespace_nowrap()
                            .child(label),
                    )
                })),
        )
}

fn logo_mark() -> impl IntoElement {
    img(panel_logo())
        .id("tokenbar-logo")
        .w(px(22.))
        .h(px(22.))
        .object_fit(ObjectFit::Contain)
        .with_fallback(|| {
            div()
                .size(px(22.))
                .rounded(px(5.))
                .bg(rgb(0x0b1b3a))
                .into_any_element()
        })
}

fn panel_logo() -> Arc<Image> {
    static LOGO: OnceLock<Arc<Image>> = OnceLock::new();
    LOGO.get_or_init(|| Arc::new(Image::from_bytes(ImageFormat::Svg, LOGO_SVG.to_vec())))
        .clone()
}

fn provider_card(
    provider: &ProviderSnapshot,
    expanded: bool,
    cx: &mut Context<Panel>,
) -> impl IntoElement {
    let face = face_for(provider);
    let id = provider.id;
    let value_color = if face.quiet_value { MUTED } else { TEXT };
    div()
        .id(row_id(id))
        .cursor_pointer()
        .flex()
        .flex_col()
        .gap(px(3.))
        .px(px(12.))
        .py(px(9.))
        .rounded_lg()
        .bg(rgb(CARD))
        .border_1()
        .border_color(rgb(LINE))
        .child(
            div()
                .flex()
                .items_center()
                .justify_between()
                .gap(px(12.))
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .gap(px(2.))
                        .flex_grow_1()
                        .child(
                            div()
                                .text_size(px(13.))
                                .font_weight(FontWeight::SEMIBOLD)
                                .child(provider.label.clone()),
                        )
                        .child(
                            div()
                                .text_size(px(11.))
                                .line_height(px(15.))
                                .text_color(rgb(MUTED))
                                .whitespace_normal()
                                .child(face.subtitle),
                        ),
                )
                .child(
                    div()
                        .text_size(px(13.))
                        .font_weight(FontWeight::SEMIBOLD)
                        .text_color(rgb(value_color))
                        .whitespace_nowrap()
                        .child(face.value),
                ),
        )
        .when_some(face.bar, |card, used| card.child(usage_bar(used)))
        .when(expanded && !face.technical.is_empty(), |card| {
            card.child(
                div()
                    .text_size(px(11.))
                    .line_height(px(15.))
                    .text_color(rgb(MUTED))
                    .whitespace_normal()
                    .child(face.technical),
            )
        })
        .on_click(cx.listener(move |_this, _event, _window, cx| toggle_expanded(id, cx)))
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

fn usage_bar(used: f64) -> impl IntoElement {
    let pct = used.clamp(0.0, 100.0) as f32;
    let color = if used >= 85.0 {
        0xff453a
    } else if used >= 60.0 {
        0xff9f0a
    } else {
        0x30d158
    };
    div()
        .flex()
        .h(px(3.))
        .w_full()
        .mt(px(4.))
        .rounded_full()
        .bg(rgb(0x3a3a3c))
        .child(
            div()
                .h(px(3.))
                .flex_grow(pct.max(0.01))
                .rounded_full()
                .bg(rgb(color)),
        )
        .child(div().h(px(3.)).flex_grow((100.0 - pct).max(0.01)))
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
                    "mode-used",
                    "已用",
                    settings.title_mode == TitleMode::UsedPercent,
                    TitleMode::UsedPercent,
                    cx,
                ))
                .child(mode_button(
                    "mode-left",
                    "剩余",
                    settings.title_mode == TitleMode::RemainingPercent,
                    TitleMode::RemainingPercent,
                    cx,
                ))
                .child(mode_button(
                    "mode-tokens",
                    "Token",
                    settings.title_mode == TitleMode::Tokens,
                    TitleMode::Tokens,
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
                .child("从剪贴板粘贴凭据，会覆盖自动读取。"),
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
            "粘贴 Claude OAuth",
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
        .child(
            div()
                .text_size(px(11.))
                .line_height(px(15.))
                .text_color(rgb(MUTED))
                .whitespace_normal()
                .child("未粘贴时，仍会读取 Cursor 登录、Claude 凭据和 Codex 日志。"),
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
    let title = {
        let shell = cx.global::<Shell>();
        shell.settings.menu_title(&shell.snapshot.providers)
    };
    let tray = cx.global::<Shell>().tray.clone();
    let _ = tray.set_title(Some(title.clone()), cx);
    let _ = tray.set_tooltip(Some(format!("TokenBar {title}")), cx);
}

fn face_for(provider: &ProviderSnapshot) -> RowFace {
    if provider.source == "pending" && provider.used_percent.is_none() && provider.tokens.is_none()
    {
        return RowFace {
            value: "…".to_string(),
            subtitle: "正在读取".to_string(),
            technical: String::new(),
            quiet_value: true,
            bar: None,
        };
    }

    let technical = technical_line(provider);
    if provider.used_percent.is_none() && provider.tokens.is_none() {
        let (value, subtitle) = empty_copy(provider.id);
        return RowFace {
            value,
            subtitle,
            technical,
            quiet_value: true,
            bar: None,
        };
    }

    let mut bits = Vec::new();
    if provider.detail.contains("沿用上次") {
        bits.push("沿用上次".to_string());
    }
    if let Some(used) = provider.used_percent {
        let remaining = provider
            .remaining_percent
            .unwrap_or_else(|| (100.0 - used).clamp(0.0, 100.0));
        bits.push(format!("剩余 {}%", format_percent(remaining)));
    }
    if let Some(reset) = provider.resets_at.as_deref() {
        let hint = reset_hint(reset);
        if !hint.is_empty() {
            bits.push(hint);
        }
    } else if provider.used_percent.is_some() {
        if let Some(tokens) = provider.tokens {
            bits.push(format!("日志 {}", format_compact(tokens)));
        }
    } else {
        bits.push("可计费，不含缓存读".to_string());
    }
    RowFace {
        value: panel_value(provider),
        subtitle: bits.join(" · "),
        technical,
        quiet_value: false,
        bar: provider.used_percent,
    }
}

fn empty_copy(id: ProviderId) -> (String, String) {
    match id {
        ProviderId::Cursor => (
            "未登录".to_string(),
            "在设置里粘贴会话，或先登录 Cursor".to_string(),
        ),
        ProviderId::Claude => (
            "未登录".to_string(),
            "在设置里粘贴 OAuth，或运行 claude /login".to_string(),
        ),
        ProviderId::Openai => (
            "暂无".to_string(),
            "登录 Codex，或在设置里粘贴 sk- 密钥".to_string(),
        ),
    }
}

fn panel_value(provider: &ProviderSnapshot) -> String {
    if let Some(used) = provider.used_percent {
        format!("{}%", format_percent(used))
    } else if let Some(tokens) = provider.tokens {
        format_compact(tokens)
    } else {
        "—".to_string()
    }
}

fn technical_line(provider: &ProviderSnapshot) -> String {
    let mut parts = Vec::new();
    if !provider.detail.is_empty()
        && provider.detail != "Refreshing"
        && provider.detail != "offline stub"
    {
        parts.push(soften(&provider.detail));
    }
    if let Some(error) = &provider.error {
        parts.push(soften(error));
    }
    let text = parts.join(" · ");
    truncate_chars(&text, 120)
}

fn soften(raw: &str) -> String {
    let mut text = raw.trim().to_string();
    if let Some(home) = std::env::var_os("HOME") {
        text = text.replace(home.to_string_lossy().as_ref(), "~");
    }
    text = text.replace("expected ", "");
    loop {
        let start = text.find("https://").or_else(|| text.find("http://"));
        let Some(start) = start else {
            break;
        };
        let rest = &text[start..];
        let end = rest
            .find(|ch: char| ch.is_whitespace() || ch == ')')
            .unwrap_or(rest.len());
        text.replace_range(start..start + end, "网络");
    }
    text
}

fn truncate_chars(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        return text.to_string();
    }
    let mut shortened: String = text.chars().take(max.saturating_sub(1)).collect();
    shortened.push('…');
    shortened
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
    }
}

fn kind_name(kind: SecretKind) -> &'static str {
    match kind {
        SecretKind::CursorSession => "Cursor 会话",
        SecretKind::ClaudeOauth => "Claude OAuth",
        SecretKind::OpenaiKey => "OpenAI 密钥",
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
    let title = cx
        .global::<Shell>()
        .settings
        .menu_title(&snapshot.providers);
    {
        let shell = cx.global_mut::<Shell>();
        shell.snapshot = snapshot;
        if shell.status == "正在刷新…" || shell.status == "正在读取" {
            shell.status.clear();
        }
    }
    let tray = cx.global::<Shell>().tray.clone();
    if let Err(err) = tray.set_title(Some(title.clone()), cx) {
        cx.global_mut::<Shell>().status = format!("托盘标题更新失败：{err}");
    }
    let _ = tray.set_tooltip(Some(format!("TokenBar {title}")), cx);
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
    let tray = Tray::builder()
        .icon(tray_icon(cx)?)
        .title("TokenBar")
        .tooltip("TokenBar")
        .on_activate_with(|_token, cx| open_panel(cx))
        .build(cx)?;
    cx.set_global(Shell {
        tray,
        window: None,
        snapshot: placeholder_snapshot(),
        status: "正在读取".to_string(),
        show_settings: false,
        expanded: None,
        saved: secret_presence(),
        settings: load_settings(),
    });
    start_refresh_loop(cx);
    eprintln!("TokenBar is running in the menu bar. Click the icon to open the panel.");
    Ok(())
}

/// macOS menu-bar apps use the accessory activation policy so they can open
/// windows without taking a Dock slot. gpui-kit 0.6 does not expose that
/// switch, so this calls AppKit directly. `LSUIElement` in the packaged
/// Info.plist covers launches of the `.app` bundle as well.
#[cfg(target_os = "macos")]
fn set_accessory() {
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};

    let Some(marker) = MainThreadMarker::new() else {
        return;
    };
    let app = NSApplication::sharedApplication(marker);
    let _ = app.setActivationPolicy(NSApplicationActivationPolicy::Accessory);
}

#[cfg(not(target_os = "macos"))]
fn set_accessory() {}

fn open_panel(cx: &mut App) {
    set_accessory();
    if let Some(window) = cx.global::<Shell>().window {
        let activated = window
            .update(cx, |_panel, window, _cx| {
                window.activate_window();
            })
            .is_ok();
        if activated {
            return;
        }
        cx.global_mut::<Shell>().window = None;
    }

    match cx.open_window(panel_options(), |_window, cx| cx.new(|_| Panel)) {
        Ok(handle) => {
            cx.global_mut::<Shell>().window = Some(handle);
            set_accessory();
        }
        Err(err) => {
            eprintln!("failed to open TokenBar panel: {err}");
            cx.global_mut::<Shell>().status = "无法打开面板".to_string();
        }
    }
}

fn panel_options() -> WindowOptions {
    WindowOptions {
        focus: true,
        show: true,
        is_resizable: true,
        window_min_size: Some(size(px(360.), px(280.))),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(180.), px(72.)),
            size: size(px(400.), px(460.)),
        })),
        titlebar: Some(TitlebarOptions {
            title: Some("TokenBar".into()),
            ..Default::default()
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

fn tray_icon(cx: &App) -> gpui_tray::Result<Icon> {
    let image = Image::from_bytes(ImageFormat::Svg, TRAY_SVG.to_vec());
    let builder = Icon::from_gpui(&image, cx);
    // Keep the navy / white / yellow mark. A template icon would flatten it.
    #[cfg(target_os = "macos")]
    let builder = builder.template(false).scale_factor(2.0);
    match builder.build() {
        Ok(icon) => Ok(icon),
        Err(err) => {
            eprintln!("SVG tray icon failed ({err}); using the ring fallback");
            ring_fallback()
        }
    }
}

fn ring_fallback() -> gpui_tray::Result<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    let center = (SIZE as f32 - 1.0) / 2.0;
    for y in 0..SIZE {
        for x in 0..SIZE {
            let px = x as f32 + 0.5;
            let py = y as f32 + 0.5;
            if !in_rounded_square(px, py, SIZE as f32, 7.0) {
                continue;
            }
            let dx = px - center;
            let dy = py - center;
            let on_ring = (dx.hypot(dy) - 11.6).abs() <= 2.6;
            let on_bar = (2.2..29.8).contains(&px) && (13.4..18.6).contains(&py);
            let (red, green, blue) = if on_bar {
                (0xF5, 0xC5, 0x18)
            } else if on_ring {
                (0xFF, 0xFF, 0xFF)
            } else {
                (0x0B, 0x1B, 0x3A)
            };
            let offset = ((y * SIZE + x) * 4) as usize;
            rgba[offset] = red;
            rgba[offset + 1] = green;
            rgba[offset + 2] = blue;
            rgba[offset + 3] = 255;
        }
    }
    let builder = Icon::from_rgba(rgba, SIZE, SIZE);
    #[cfg(target_os = "macos")]
    let builder = builder.template(false).scale_factor(2.0);
    builder.build()
}

fn in_rounded_square(x: f32, y: f32, size: f32, radius: f32) -> bool {
    let cx = x.clamp(radius, size - radius);
    let cy = y.clamp(radius, size - radius);
    let dx = x - cx;
    let dy = y - cy;
    dx * dx + dy * dy <= radius * radius
}
