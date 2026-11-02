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
    delete_secret, placeholder_snapshot, save_secret, secret_presence, ProviderId,
    ProviderSnapshot, SecretKind, SecretPresence, UsageSnapshot,
};

const REFRESH_EVERY: Duration = Duration::from_secs(300);
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
}

impl Global for Shell {}

struct Panel;

struct RowFace {
    value: String,
    subtitle: String,
    technical: String,
    quiet_value: bool,
}

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (generated, status, show_settings, saved, providers, expanded) = {
            let shell = cx.global::<Shell>();
            (
                shell.snapshot.generated_at.clone(),
                shell.status.clone(),
                shell.show_settings,
                shell.saved,
                shell.snapshot.providers.clone(),
                shell.expanded,
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
            .child(header(&subtitle, &providers))
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
                    .when(show_settings, |parent| {
                        parent.child(settings_card(saved, cx))
                    }),
            )
            .child(footer(cx))
    }
}

fn header(subtitle: &str, providers: &[ProviderSnapshot]) -> impl IntoElement {
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
                .children(providers.iter().map(|provider| {
                    div()
                        .text_size(px(11.))
                        .text_color(rgb(MUTED))
                        .whitespace_nowrap()
                        .child(provider.meter())
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
                .rounded_full()
                .border_2()
                .border_color(rgb(ACCENT))
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
        .on_click(cx.listener(move |_this, _event, _window, cx| toggle_row(id, cx)))
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

fn settings_card(saved: SecretPresence, cx: &mut Context<Panel>) -> impl IntoElement {
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

fn face_for(provider: &ProviderSnapshot) -> RowFace {
    if provider.source == "pending" {
        return RowFace {
            value: "…".to_string(),
            subtitle: "正在读取".to_string(),
            technical: String::new(),
            quiet_value: true,
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
        };
    }

    let value = panel_value(provider);
    let subtitle = if provider.used_percent.is_some() {
        match (&provider.plan, provider.error.is_some()) {
            (Some(plan), false) => format!("额度已用 · {plan}"),
            (_, true) => "额度已用 · 在线数据不完整".to_string(),
            (None, false) => "额度已用".to_string(),
        }
    } else if provider.error.is_some() {
        "本地日志 · 在线额度暂不可用".to_string()
    } else {
        "本地日志".to_string()
    };
    RowFace {
        value,
        subtitle,
        technical,
        quiet_value: false,
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
    let meter = provider.meter();
    let rest = meter.trim_start_matches(|ch: char| ch.is_ascii_alphabetic());
    if rest.is_empty() || rest == "--" {
        "—".to_string()
    } else {
        rest.to_string()
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

fn toggle_row(id: ProviderId, cx: &mut Context<Panel>) {
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

fn apply_snapshot(cx: &mut App, snapshot: UsageSnapshot) {
    let title = snapshot.title.clone();
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
        .title("R-- C-- O--")
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
        cx.background_executor().timer(REFRESH_EVERY).await;
    })
    .detach();
}

fn tray_icon(cx: &App) -> gpui_tray::Result<Icon> {
    let image = Image::from_bytes(ImageFormat::Svg, TRAY_SVG.to_vec());
    let builder = Icon::from_gpui(&image, cx);
    #[cfg(target_os = "macos")]
    let builder = builder.template(true).scale_factor(2.0);
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
            let dx = x as f32 - center;
            let dy = y as f32 - center;
            let distance = dx.hypot(dy);
            let offset = ((y * SIZE + x) * 4) as usize;
            let mut alpha = 0u8;
            if (9.2..13.4).contains(&distance) {
                let angle = dy.atan2(dx);
                let mut from_top = angle + std::f32::consts::FRAC_PI_2;
                if from_top < 0.0 {
                    from_top += std::f32::consts::TAU;
                }
                alpha = if from_top < 4.7 { 255 } else { 80 };
            } else if dx.abs() <= 4.6 && dy.abs() <= 4.6 {
                alpha = 255;
            }
            rgba[offset] = 0;
            rgba[offset + 1] = 0;
            rgba[offset + 2] = 0;
            rgba[offset + 3] = alpha;
        }
    }
    let builder = Icon::from_rgba(rgba, SIZE, SIZE);
    #[cfg(target_os = "macos")]
    let builder = builder.template(true).scale_factor(2.0);
    builder.build()
}
