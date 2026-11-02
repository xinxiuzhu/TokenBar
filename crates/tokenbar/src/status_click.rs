//! macOS status-item clicks.
//!
//! gpui-tray delivers a primary click only when AppKit sends the status
//! button action. Accessory apps often swallow that click, so the panel never
//! appears. This installs the same split AI-Usage uses: left click toggles
//! the panel, right click (and control-click) opens Refresh / Settings / Quit.
//!
//! The monitor consumes status-bar mouse-downs so the button action cannot
//! toggle the panel a second time.

use std::ptr::NonNull;
use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Sel};
use objc2::{define_class, msg_send, sel, DefinedClass, MainThreadMarker, MainThreadOnly};
use objc2_app_kit::{
    NSEvent, NSEventMask, NSEventModifierFlags, NSEventType, NSMenu, NSMenuItem, NSScreen, NSWindow,
};
use objc2_foundation::{NSObject, NSObjectProtocol, NSString};

use crate::ui::{StatusAnchor, TrayCommand};

static PANEL_OPEN: AtomicBool = AtomicBool::new(false);

pub fn set_panel_open(open: bool) {
    PANEL_OPEN.store(open, Ordering::Release);
}

struct ClickIvars {
    tx: async_channel::Sender<TrayCommand>,
}

define_class!(
    #[unsafe(super = NSObject)]
    #[thread_kind = MainThreadOnly]
    #[ivars = ClickIvars]
    struct StatusMenuTarget;

    unsafe impl NSObjectProtocol for StatusMenuTarget {}

    impl StatusMenuTarget {
        #[unsafe(method(tokenbarRefresh:))]
        fn refresh(&self, _sender: &AnyObject) {
            let _ = self.ivars().tx.try_send(TrayCommand::Refresh);
        }

        #[unsafe(method(tokenbarSettings:))]
        fn settings(&self, _sender: &AnyObject) {
            let _ = self.ivars().tx.try_send(TrayCommand::Settings);
        }

        #[unsafe(method(tokenbarQuit:))]
        fn quit(&self, _sender: &AnyObject) {
            let _ = self.ivars().tx.try_send(TrayCommand::Quit);
        }
    }
);

impl StatusMenuTarget {
    fn new(mtm: MainThreadMarker, tx: async_channel::Sender<TrayCommand>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ClickIvars { tx });
        unsafe { msg_send![super(this), init] }
    }
}

pub fn install(tx: async_channel::Sender<TrayCommand>) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let target = StatusMenuTarget::new(mtm, tx.clone());
    let menu_target = target.clone();
    let click_tx = tx.clone();
    let local = block2::RcBlock::new(move |event: NonNull<NSEvent>| -> *mut NSEvent {
        let event = unsafe { event.as_ref() };
        let Some(mtm) = MainThreadMarker::new() else {
            return pass(event);
        };
        let Some(window) = event.window(mtm) else {
            return pass(event);
        };
        if !is_status_bar_window(&window) {
            return pass(event);
        }
        let frame = window.frame();
        let anchor = StatusAnchor {
            x: frame.origin.x,
            y: frame.origin.y,
            width: frame.size.width,
            height: frame.size.height,
        };
        let control = event
            .modifierFlags()
            .contains(NSEventModifierFlags::Control);
        let right = event.r#type() == NSEventType::RightMouseDown;
        if right || control {
            show_menu(mtm, &menu_target, frame.origin.x, frame.origin.y);
            return std::ptr::null_mut();
        }
        let _ = click_tx.try_send(TrayCommand::Toggle(Some(anchor)));
        std::ptr::null_mut()
    });
    let mask = NSEventMask::LeftMouseDown | NSEventMask::RightMouseDown;
    let monitor = unsafe { NSEvent::addLocalMonitorForEventsMatchingMask_handler(mask, &local) };

    let dismiss_tx = tx;
    let global_block = block2::RcBlock::new(move |_event: NonNull<NSEvent>| {
        if PANEL_OPEN.load(Ordering::Acquire) {
            let _ = dismiss_tx.try_send(TrayCommand::Dismiss);
        }
    });
    let global = unsafe {
        NSEvent::addGlobalMonitorForEventsMatchingMask_handler(
            NSEventMask::LeftMouseDown,
            &global_block,
        )
    };

    // The monitors and the menu target have to outlive every click.
    std::mem::forget(local);
    std::mem::forget(monitor);
    std::mem::forget(global_block);
    std::mem::forget(global);
    std::mem::forget(target);
}

fn pass(event: &NSEvent) -> *mut NSEvent {
    event as *const NSEvent as *mut NSEvent
}

fn is_status_bar_window(window: &NSWindow) -> bool {
    let text: Retained<NSString> = unsafe { msg_send![window, description] };
    text.to_string().contains("StatusBar")
}

fn show_menu(mtm: MainThreadMarker, target: &StatusMenuTarget, x: f64, y: f64) {
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str("TokenBar"));
    add_item(mtm, &menu, target, "刷新", sel!(tokenbarRefresh:));
    add_item(mtm, &menu, target, "设置", sel!(tokenbarSettings:));
    menu.addItem(&NSMenuItem::separatorItem(mtm));
    add_item(mtm, &menu, target, "退出", sel!(tokenbarQuit:));
    let origin = objc2_foundation::NSPoint::new(x, y - 4.0);
    menu.popUpMenuPositioningItem_atLocation_inView(None, origin, None);
}

fn add_item(
    mtm: MainThreadMarker,
    menu: &NSMenu,
    target: &StatusMenuTarget,
    title: &str,
    action: Sel,
) {
    let item = unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(""),
        )
    };
    let any: &AnyObject = target;
    unsafe { item.setTarget(Some(any)) }
    menu.addItem(&item);
}

/// Top-left of the panel in GPUI coordinates, just under the status item.
/// The panel's right edge lines up with the status item, which lives on the
/// right side of the menu bar.
pub fn origin_under(anchor: &StatusAnchor, panel_width: f32) -> Option<(f32, f32)> {
    let mtm = MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    if screens.count() == 0 {
        return None;
    }
    let primary = screens.objectAtIndex(0);
    let frame = primary.frame();
    let panel_top = anchor.y - 6.0;
    let gpui_y = (frame.origin.y + frame.size.height - panel_top) as f32;
    let item_right = anchor.x + anchor.width;
    let mut gpui_x = (item_right - frame.origin.x) as f32 - panel_width;
    if gpui_x < 8.0 {
        gpui_x = 8.0;
    }
    let max_x = frame.size.width as f32 - panel_width - 8.0;
    if max_x.is_finite() && gpui_x > max_x {
        gpui_x = max_x.max(8.0);
    }
    Some((gpui_x, gpui_y.max(4.0)))
}
