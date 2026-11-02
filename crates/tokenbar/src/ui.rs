//! GPUI panel and gpui-tray status item.
//!
//! Primary click opens the panel. The panel is the place for Refresh,
//! Settings, and Quit. There is no native menu, because on macOS a menu
//! would consume the click that should open the panel.
//!
//! macOS runs with the accessory activation policy so the app stays out of
//! the Dock. `packaging/macos/package.sh` also sets `LSUIElement`.

use std::time::Duration;

use gpui_kit::prelude::*;
use gpui_kit::{
    div, point, px, rgb, size, App, Bounds, ClipboardItem, Context, Global, QuitMode, Render,
    TitlebarOptions, Window, WindowBounds, WindowHandle, WindowOptions,
};
use gpui_tray::{Icon, Tray};
use tokenbar_core::{
    delete_secret, placeholder_snapshot, save_secret, secret_presence, ProviderSnapshot,
    SecretKind, SecretPresence, UsageSnapshot,
};

const REFRESH_EVERY: Duration = Duration::from_secs(300);

struct Shell {
    tray: Tray,
    window: Option<WindowHandle<Panel>>,
    snapshot: UsageSnapshot,
    status: String,
    show_settings: bool,
    saved: SecretPresence,
}

impl Global for Shell {}

struct Panel;

impl Render for Panel {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let (title, generated, status, show_settings, saved, providers) = {
            let shell = cx.global::<Shell>();
            (
                shell.snapshot.title.clone(),
                shell.snapshot.generated_at.clone(),
                shell.status.clone(),
                shell.show_settings,
                shell.saved,
                shell.snapshot.providers.clone(),
            )
        };

        div()
            .id("tokenbar-root")
            .size_full()
            .flex()
            .flex_col()
            .gap(px(12.))
            .p(px(16.))
            .bg(rgb(0x12141a))
            .text_color(rgb(0xf4f4f5))
            .overflow_y_scroll()
            .child(header(&title, &generated))
            .children(providers.iter().map(provider_card))
            .child(button_row(cx))
            .when(show_settings, |parent| {
                parent.child(settings_card(saved, cx))
            })
            .child(
                div()
                    .text_size(px(12.))
                    .text_color(rgb(0xa1a1aa))
                    .child(status),
            )
    }
}

fn header(title: &str, generated: &str) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .child(
            div()
                .text_size(px(13.))
                .text_color(rgb(0xa1a1aa))
                .child("TokenBar"),
        )
        .child(div().text_size(px(22.)).child(title.to_string()))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xa1a1aa))
                .child(format!("Updated {generated}")),
        )
}

fn provider_card(provider: &ProviderSnapshot) -> impl IntoElement + use<'_> {
    let mut body = div()
        .flex()
        .flex_col()
        .gap(px(4.))
        .p(px(12.))
        .rounded(px(8.))
        .bg(rgb(0x1c1e27))
        .child(
            div()
                .flex()
                .justify_between()
                .child(provider.label.clone())
                .child(provider.meter()),
        )
        .child(div().text_size(px(12.)).text_color(rgb(0xa1a1aa)).child(
            if provider.detail.is_empty() {
                provider.source.clone()
            } else {
                format!("{} · {}", provider.source, provider.detail)
            },
        ));
    if let Some(error) = &provider.error {
        body = body.child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xf0a8a8))
                .child(error.clone()),
        );
    }
    body
}

fn button_row(cx: &mut Context<Panel>) -> impl IntoElement {
    div()
        .flex()
        .gap(px(8.))
        .child(action_button(
            "refresh",
            "Refresh",
            rgb(0x2563eb),
            cx,
            |_, _, cx| {
                refresh_now(cx);
            },
        ))
        .child(action_button(
            "settings",
            "Settings",
            rgb(0x27272f),
            cx,
            |_, _, cx| {
                let open = !cx.global::<Shell>().show_settings;
                cx.global_mut::<Shell>().show_settings = open;
                cx.notify();
            },
        ))
        .child(action_button(
            "quit",
            "Quit",
            rgb(0x3f1d1d),
            cx,
            |_, _, cx| {
                quit(cx);
            },
        ))
}

fn action_button<F>(
    id: &'static str,
    label: &'static str,
    fill: impl Into<gpui_kit::Hsla> + Clone + 'static,
    cx: &mut Context<Panel>,
    on_click: F,
) -> impl IntoElement
where
    F: Fn(&mut Panel, &mut Window, &mut Context<Panel>) + 'static,
{
    let fill = fill.into();
    div()
        .id(id)
        .cursor_pointer()
        .px(px(12.))
        .py(px(6.))
        .rounded(px(6.))
        .bg(fill)
        .child(label)
        .on_click(cx.listener(move |this, _event, window, cx| on_click(this, window, cx)))
}

fn settings_card(saved: SecretPresence, cx: &mut Context<Panel>) -> impl IntoElement {
    div()
        .flex()
        .flex_col()
        .gap(px(8.))
        .p(px(12.))
        .rounded(px(8.))
        .bg(rgb(0x1c1e27))
        .child("Paste a credential from the clipboard. It overrides auto-detected files.")
        .child(secret_row(
            "paste-cursor",
            "clear-cursor",
            "Paste Cursor session",
            saved.cursor,
            SecretKind::CursorSession,
            cx,
        ))
        .child(secret_row(
            "paste-claude",
            "clear-claude",
            "Paste Claude OAuth",
            saved.claude,
            SecretKind::ClaudeOauth,
            cx,
        ))
        .child(secret_row(
            "paste-openai",
            "clear-openai",
            "Paste OpenAI key",
            saved.openai,
            SecretKind::OpenaiKey,
            cx,
        ))
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xa1a1aa))
                .child(
                    "Cursor also reads state.vscdb. Claude reads ~/.claude/.credentials.json (and the macOS keychain). OpenAI reads ~/.codex/auth.json plus Codex JSONL. A pasted sk- key adds organization usage.",
                ),
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
                .py(px(6.))
                .rounded(px(6.))
                .bg(rgb(0x27272f))
                .child(label)
                .on_click(cx.listener(move |_this, _event, _window, cx| paste(kind, cx))),
        )
        .child(
            div()
                .text_size(px(12.))
                .text_color(rgb(0xa1a1aa))
                .child(if saved { "saved" } else { "not set" }),
        )
        .child(
            div()
                .id(clear_id)
                .cursor_pointer()
                .text_size(px(12.))
                .child("Clear")
                .on_click(cx.listener(move |_this, _event, _window, cx| clear_secret(kind, cx))),
        )
}

fn paste(kind: SecretKind, cx: &mut Context<Panel>) {
    let Some(item) = cx.read_from_clipboard() else {
        set_status(cx, "Clipboard is empty");
        return;
    };
    let Some(text) = clipboard_text(&item) else {
        set_status(cx, "Clipboard has no text");
        return;
    };
    match save_secret(kind, &text) {
        Ok(()) => {
            refresh_saved(cx);
            set_status(cx, &format!("{} saved", kind.label()));
            refresh_now(cx);
        }
        Err(err) => set_status(cx, &err),
    }
}

fn clear_secret(kind: SecretKind, cx: &mut Context<Panel>) {
    match delete_secret(kind) {
        Ok(()) => {
            refresh_saved(cx);
            set_status(cx, &format!("{} cleared", kind.label()));
        }
        Err(err) => set_status(cx, &err),
    }
}

fn clipboard_text(item: &ClipboardItem) -> Option<String> {
    let text = item.text()?.to_string();
    let text = text.trim();
    if text.is_empty() {
        None
    } else {
        Some(text.to_string())
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
    cx.global_mut::<Shell>().status = "Refreshing…".to_string();
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
    let generated = snapshot.generated_at.clone();
    {
        let shell = cx.global_mut::<Shell>();
        shell.snapshot = snapshot;
        shell.status = format!("Updated {generated}");
    }
    let tray = cx.global::<Shell>().tray.clone();
    if let Err(err) = tray.set_title(Some(title.clone()), cx) {
        cx.global_mut::<Shell>().status = format!("Updated {generated}, tray title failed: {err}");
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
        .icon(tray_icon()?)
        .title("R-- C-- O--")
        .tooltip("TokenBar")
        .on_activate_with(|_token, cx| open_panel(cx))
        .build(cx)?;
    cx.set_global(Shell {
        tray,
        window: None,
        snapshot: placeholder_snapshot(),
        status: "Starting".to_string(),
        show_settings: false,
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

    let opened = cx.open_window(panel_options(), |_window, cx| cx.new(|_| Panel));
    match opened {
        Ok(handle) => {
            cx.global_mut::<Shell>().window = Some(handle);
            set_accessory();
        }
        Err(err) => {
            eprintln!("failed to open TokenBar panel: {err}");
            cx.global_mut::<Shell>().status = format!("Could not open the panel: {err}");
        }
    }
}

fn panel_options() -> WindowOptions {
    WindowOptions {
        focus: true,
        show: true,
        is_resizable: true,
        window_min_size: Some(size(px(380.), px(320.))),
        window_bounds: Some(WindowBounds::Windowed(Bounds {
            origin: point(px(160.), px(80.)),
            size: size(px(460.), px(560.)),
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

fn tray_icon() -> gpui_tray::Result<Icon> {
    const SIZE: u32 = 32;
    let mut rgba = vec![0_u8; (SIZE * SIZE * 4) as usize];
    for y in 0..SIZE {
        for x in 0..SIZE {
            let offset = ((y * SIZE + x) * 4) as usize;
            let dx = x as i32 - 16;
            let dy = y as i32 - 16;
            let inside = dx * dx + dy * dy <= 13 * 13;
            let stem = (13..=18).contains(&x) && (8..=24).contains(&y);
            let bar = (8..=23).contains(&x) && (8..=12).contains(&y);
            if stem || bar {
                rgba[offset..offset + 4].copy_from_slice(&[255, 255, 255, 255]);
            } else if inside {
                rgba[offset..offset + 4].copy_from_slice(&[37, 99, 235, 255]);
            }
        }
    }
    Icon::from_rgba(rgba, SIZE, SIZE).build()
}
