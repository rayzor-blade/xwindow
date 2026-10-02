//! A window in a page: the page's canvas. The program asks its host to
//! start the `xwindow` agent (agent.mjs) on the page's thread, where the
//! canvas's events fire, with the address of a block holding a mailbox and
//! a queue. What the program asks of the window is encoded onto the wire
//! (`crate::wire`) as the DOM's own operations, or the agent's (`XwAgent`),
//! and sent through the mailbox when the program next polls. The agent
//! posts each event to the queue as an `XwEvent`, which `poll` reads
//! without a call. A redraw request is a word the agent reads each frame.
//!
//! A page has one window. What a page cannot do, such as place or decorate
//! it, does nothing; what it has no answer for reads as the default.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, Ordering::SeqCst};
use std::sync::{LazyLock, Mutex, MutexGuard};

use xwindow_core::{Kind, Slab};

use crate::runtime::{Buffer, ErrorKind, Text, host};
use crate::wire::{self, Handle, Mailbox, Queue};
use crate::{
    CursorGrab, CursorIcon, Event, Key, KeyCode, KeyLocation, MouseButton, ScaleSizing, ScrollUnit,
    Theme, TouchPhase, WindowAttributes,
};

/// The name a host starts the agent by: it imports `xwindow.mjs` beside the
/// program and calls its `start({ memory, address, canvas })`.
const AGENT_NAME: &str = "xwindow";

// The page's objects, under the handles the agent keeps them by.
const DOCUMENT: Handle = Handle(2);
const STYLE: Handle = Handle(4);
const AGENT: Handle = Handle(5);

/// What the agent is started with. The agent reads the mailbox at its
/// address, the queue 24 bytes on and the redraw word 44 bytes on.
#[repr(C)]
struct Block {
    mailbox: Mailbox,
    queue: Queue,
    /// Set by the program, cleared by the agent when it posts the redraw.
    redraw: AtomicI32,
}

static BLOCK: Block = Block {
    mailbox: Mailbox::new(),
    queue: Queue::new(),
    redraw: AtomicI32::new(0),
};

/// Room in the queue for a burst of pointer events between two polls.
const RING: usize = 1 << 16;

/// The screen, the one monitor a page has.
const SCREEN: i32 = (Kind::Monitor as i32) << 26 | 1;

struct Page {
    windows: Slab<()>,
    /// Whether the host started the agent, which then serves the page for
    /// the program's life.
    started: bool,
    ring: Box<[u8]>,
    commands: wire::Encoder,
    staged: Vec<Box<[u8]>>,
    events: VecDeque<Event>,
    width: i32,
    height: i32,
    scale: f64,
    focused: bool,
    visible: bool,
    fullscreen: bool,
    dark: bool,
    title: String,
    sizing: ScaleSizing,
    /// The cursor's CSS, and whether it shows.
    cursor: String,
    cursor_visible: bool,
    screen: (i32, i32, String),
}

static PAGE: LazyLock<Mutex<Page>> = LazyLock::new(|| {
    Mutex::new(Page {
        windows: Slab::new(Kind::Window),
        started: false,
        ring: vec![0; RING].into_boxed_slice(),
        commands: wire::Encoder::new(),
        staged: Vec::new(),
        events: VecDeque::new(),
        width: 0,
        height: 0,
        scale: 1.0,
        focused: false,
        visible: true,
        fullscreen: false,
        dark: false,
        title: String::new(),
        sizing: ScaleSizing::Logical,
        cursor: "default".to_owned(),
        cursor_visible: true,
        screen: (0, 0, String::new()),
    })
});

fn page() -> MutexGuard<'static, Page> {
    PAGE.lock().unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl Page {
    /// Hand the commands to the agent and wait until it has run them.
    fn flush(&mut self) {
        if !self.commands.bytes.is_empty() {
            BLOCK.mailbox.send(&self.commands.bytes);
            self.commands.bytes.clear();
            self.staged.clear();
        }
    }

    /// Everything the agent has posted, as events.
    fn drain(&mut self) {
        while let Some(record) = BLOCK.queue.take::<wire::XwEvent>(&self.ring) {
            if let Some(event) = self.event(record) {
                self.events.push_back(event);
            }
        }
    }

    fn next(&mut self, handle: i32, timeout: Option<i64>) -> Event {
        if self.windows.get(handle).is_none() {
            return Event::None;
        }
        self.flush();
        if self.events.is_empty() {
            let seen = BLOCK.queue.tail();
            self.drain();
            if let (true, Some(timeout)) = (self.events.is_empty(), timeout) {
                BLOCK.queue.wait(seen, timeout);
                self.drain();
            }
        }
        self.events.pop_front().unwrap_or_default()
    }

    fn set_cursor(&mut self) {
        let css = if self.cursor_visible {
            self.cursor.clone()
        } else {
            "none".to_owned()
        };
        self.commands
            .css_style_declaration_set_property(STYLE, &"cursor".to_owned(), &css, &None);
    }

    fn event(&mut self, e: wire::XwEvent) -> Option<Event> {
        use wire::XwEventKind as K;
        let device = e.device.unwrap_or(0);
        let x = e.x.unwrap_or(0.0);
        let y = e.y.unwrap_or(0.0);
        let on = e.on.unwrap_or(false);
        let text = e.text.clone().unwrap_or_default();
        Some(match e.kind {
            K::Resized => {
                self.width = e.width.unwrap_or(0) as i32;
                self.height = e.height.unwrap_or(0) as i32;
                if let Some(scale) = e.scale {
                    self.scale = scale;
                }
                Event::Resized {
                    width: self.width,
                    height: self.height,
                }
            }
            K::Focused => {
                self.focused = on;
                Event::Focused { focused: on }
            }
            K::Occluded => Event::Occluded { occluded: on },
            K::CloseRequested => Event::CloseRequested,
            K::RedrawRequested => Event::RedrawRequested,
            K::ScaleFactorChanged => {
                let scale = e.scale.unwrap_or(1.0);
                if self.sizing == ScaleSizing::Physical && scale > 0.0 && self.width > 0 {
                    // The same pixels at the new scale: a CSS size to match.
                    self.commands.xw_agent_set_size(
                        AGENT,
                        &(f64::from(self.width) / scale),
                        &(f64::from(self.height) / scale),
                    );
                }
                self.scale = scale;
                Event::ScaleFactorChanged { scaleFactor: scale }
            }
            K::ThemeChanged => {
                self.dark = on;
                Event::ThemeChanged {
                    theme: if on { Theme::Dark } else { Theme::Light },
                }
            }
            K::CursorEntered => Event::CursorEntered { device },
            K::CursorLeft => Event::CursorLeft { device },
            K::CursorMoved => Event::CursorMoved { device, x, y },
            K::MouseInput => {
                let code = i32::from(e.button.unwrap_or(0));
                let button = match code {
                    0 => MouseButton::Left,
                    1 => MouseButton::Middle,
                    2 => MouseButton::Right,
                    3 => MouseButton::Back,
                    4 => MouseButton::Forward,
                    _ => MouseButton::Other,
                };
                Event::MouseInput {
                    device,
                    button,
                    code: if button == MouseButton::Other {
                        code
                    } else {
                        0
                    },
                    pressed: on,
                }
            }
            // A browser's delta points the way the page scrolls; winit's, the
            // way the content moves.
            K::MouseWheel => Event::MouseWheel {
                device,
                unit: if e.unit.unwrap_or(0) == 0 {
                    ScrollUnit::Pixel
                } else {
                    ScrollUnit::Line
                },
                x: -x,
                y: -y,
                phase: TouchPhase::Moved,
            },
            K::KeyboardInput => {
                let code_name = e.code.clone().unwrap_or_default();
                let (key, character) = logical_key(e.key.as_deref().unwrap_or(""));
                let text = match key {
                    Key::Character if on => character.clone(),
                    Key::Space if on => " ".to_owned(),
                    _ => String::new(),
                };
                Event::KeyboardInput {
                    device,
                    code: key_code(&code_name),
                    scancode: 0,
                    key,
                    character,
                    text,
                    location: match e.location.unwrap_or(0) {
                        1 => KeyLocation::Left,
                        2 => KeyLocation::Right,
                        3 => KeyLocation::Numpad,
                        _ => KeyLocation::Standard,
                    },
                    pressed: on,
                    repeat: e.repeat.unwrap_or(false),
                    synthetic: e.synthetic.unwrap_or(false),
                }
            }
            K::ModifiersChanged => {
                let m = e.modifiers.unwrap_or(0);
                Event::ModifiersChanged {
                    shift: m & 1 != 0,
                    control: m & 2 != 0,
                    alt: m & 4 != 0,
                    superKey: m & 8 != 0,
                }
            }
            K::ImeEnabled => Event::ImeEnabled,
            K::ImePreedit => Event::ImePreedit {
                text,
                start: e.start.unwrap_or(-1),
                end: e.end.unwrap_or(-1),
            },
            K::ImeCommit => Event::ImeCommit { text },
            K::ImeDisabled => Event::ImeDisabled,
            K::HoveredFile => Event::HoveredFile { path: text },
            K::DroppedFile => Event::DroppedFile { path: text },
            K::HoveredFileCancelled => Event::HoveredFileCancelled,
            K::Touch => Event::Touch {
                device,
                id: i64::from(device),
                phase: touch_phase(e.phase.unwrap_or(1)),
                x,
                y,
                force: e.pressure.unwrap_or(-1.0),
            },
            K::PinchGesture => Event::PinchGesture {
                device,
                delta: e.scale.unwrap_or(0.0),
                phase: touch_phase(e.phase.unwrap_or(1)),
            },
            K::TouchpadPressure => Event::TouchpadPressure {
                device,
                pressure: e.pressure.unwrap_or(0.0),
                stage: i64::from(e.stage.unwrap_or(0)),
            },
            K::MouseMotion => Event::MouseMotion { device, x, y },
            K::Suspended => Event::Suspended,
            K::Resumed => Event::Resumed,
            K::FullscreenChanged => {
                self.fullscreen = on;
                return None;
            }
            K::Screen => {
                self.screen = (
                    e.width.unwrap_or(0) as i32,
                    e.height.unwrap_or(0) as i32,
                    text,
                );
                return None;
            }
        })
    }
}

fn touch_phase(phase: u16) -> TouchPhase {
    match phase {
        0 => TouchPhase::Started,
        2 => TouchPhase::Ended,
        3 => TouchPhase::Cancelled,
        _ => TouchPhase::Moved,
    }
}

/// A key by its `KeyboardEvent.code` name.
fn key_code(name: &str) -> KeyCode {
    macro_rules! codes {
        ($($name:ident),* $(,)?) => {
            match name {
                $(stringify!($name) => KeyCode::$name,)*
                _ => KeyCode::Unidentified,
            }
        };
    }
    xwindow_core::key_codes!(codes)
}

/// A key by its `KeyboardEvent.key` value, and the text a character or
/// unnamed key carries.
fn logical_key(key: &str) -> (Key, String) {
    macro_rules! named {
        ($($name:ident),* $(,)?) => {
            match key {
                $(stringify!($name) => return (Key::$name, String::new()),)*
                _ => {}
            }
        };
    }
    match key {
        " " => return (Key::Space, String::new()),
        "Dead" => return (Key::Dead, String::new()),
        "Unidentified" | "" => return (Key::Unidentified, String::new()),
        _ => {}
    }
    xwindow_core::named_keys!(named);
    // A key value is a named key or the characters the key types.
    if key.chars().count() <= 2 {
        (Key::Character, key.to_owned())
    } else {
        (Key::Unidentified, key.to_owned())
    }
}

fn cursor_css(icon: CursorIcon) -> String {
    macro_rules! icons {
        ($($name:ident),* $(,)?) => {
            match icon {
                $(CursorIcon::$name => xwindow_core::cursor_css(stringify!($name)),)*
            }
        };
    }
    xwindow_core::cursor_icons!(icons)
}

/// Runs `body` on the page if `handle` is its open window, or returns
/// `miss`.
fn with<T>(handle: i32, miss: T, body: impl FnOnce(&mut Page) -> T) -> T {
    let mut p = page();
    if p.windows.get(handle).is_none() {
        return miss;
    }
    body(&mut p)
}

// -- windows ----------------------------------------------------------------

pub unsafe fn window_open(a: &WindowAttributes) -> i32 {
    let mut p = page();
    if p.windows.iter().next().is_some() {
        host::raise(
            ErrorKind::Runtime,
            "window: a page has one window, its canvas",
        );
        return 0;
    }
    if !p.started {
        let Page { ring, .. } = &mut *p;
        BLOCK.queue.attach(ring);
        if !host::agent(AGENT_NAME, &BLOCK as *const Block as usize) {
            host::raise(
                ErrorKind::Runtime,
                "window: this program's host starts no window agent",
            );
            return 0;
        }
        p.started = true;
    } else {
        // Reopened after `close`, which hid the canvas.
        p.visible = true;
        p.commands.xw_agent_set_visible(AGENT, &true);
    }
    let handle = p.windows.put(());
    p.sizing = a
        .scaleSizing
        .and_then(ScaleSizing::from_native)
        .unwrap_or_default();
    if let Some(title) = &a.title {
        let title = title.get().as_str().to_owned();
        p.commands.document_set_title(DOCUMENT, &title);
        p.title = title;
    }
    let css = |v: Option<i32>| f64::from(v.unwrap_or(0).max(0));
    if a.width.is_some() || a.height.is_some() {
        p.commands
            .xw_agent_set_size(AGENT, &css(a.width), &css(a.height));
    }
    if a.minWidth.is_some() || a.minHeight.is_some() {
        p.commands
            .xw_agent_set_min_size(AGENT, &css(a.minWidth), &css(a.minHeight));
    }
    if a.maxWidth.is_some() || a.maxHeight.is_some() {
        p.commands
            .xw_agent_set_max_size(AGENT, &css(a.maxWidth), &css(a.maxHeight));
    }
    if let Some(visible) = a.visible {
        p.visible = visible;
        p.commands.xw_agent_set_visible(AGENT, &visible);
    }
    if a.fullscreen == Some(true) {
        p.commands.xw_agent_set_fullscreen(AGENT, &true);
    }
    p.flush();
    // The agent posts the canvas's size, scale, focus and theme as it
    // starts; wait a moment for them, so the window's size is known.
    for _ in 0..10 {
        let seen = BLOCK.queue.tail();
        p.drain();
        if p.width > 0 {
            break;
        }
        BLOCK.queue.wait(seen, 100_000_000);
    }
    handle
}

pub unsafe fn window_valid(handle: i32) -> bool {
    page().windows.get(handle).is_some()
}

pub unsafe fn window_poll(handle: i32) -> Event {
    page().next(handle, None)
}

pub unsafe fn window_wait(handle: i32, timeout: f64) -> Event {
    let timeout = if timeout < 0.0 {
        -1
    } else {
        (timeout * 1e9).min(i64::MAX as f64) as i64
    };
    page().next(handle, Some(timeout))
}

/// A page's canvas cannot close: it is hidden, and polls nothing more.
pub unsafe fn window_close(handle: i32) {
    with(handle, (), |p| {
        p.commands.xw_agent_set_visible(AGENT, &false);
        p.flush();
        p.windows.remove(handle);
        p.events.clear();
    });
}

pub unsafe fn window_width(handle: i32) -> i32 {
    with(handle, 0, |p| p.width)
}

pub unsafe fn window_height(handle: i32) -> i32 {
    with(handle, 0, |p| p.height)
}

pub unsafe fn window_outer_width(handle: i32) -> i32 {
    with(handle, 0, |p| p.width)
}

pub unsafe fn window_outer_height(handle: i32) -> i32 {
    with(handle, 0, |p| p.height)
}

pub unsafe fn window_x(_: i32) -> i32 {
    0
}

pub unsafe fn window_y(_: i32) -> i32 {
    0
}

pub unsafe fn window_scale_factor(handle: i32) -> f64 {
    with(handle, 1.0, |p| p.scale)
}

pub unsafe fn window_title(handle: i32) -> Text {
    Text::new(&with(handle, String::new(), |p| p.title.clone()))
}

pub unsafe fn window_has_focus(handle: i32) -> bool {
    with(handle, false, |p| p.focused)
}

pub unsafe fn window_is_visible(handle: i32) -> bool {
    with(handle, false, |p| p.visible)
}

pub unsafe fn window_is_minimized(_: i32) -> bool {
    false
}

pub unsafe fn window_is_maximized(_: i32) -> bool {
    false
}

pub unsafe fn window_is_fullscreen(handle: i32) -> bool {
    with(handle, false, |p| p.fullscreen)
}

pub unsafe fn window_is_resizable(_: i32) -> bool {
    false
}

pub unsafe fn window_is_decorated(_: i32) -> bool {
    false
}

pub unsafe fn window_theme(handle: i32) -> i32 {
    with(handle, false, |p| p.dark)
        .then_some(Theme::Dark)
        .unwrap_or(Theme::Light)
        .native()
}

/// The canvas, which a GPU library's web surface draws to whatever handles
/// it is given.
pub unsafe fn window_platform(handle: i32) -> i32 {
    with(handle, xwindow_core::raw::NONE, |_| {
        xwindow_core::raw::WEB_CANVAS
    })
}

pub unsafe fn window_raw(_: i32, _: i32) -> i64 {
    0
}

pub unsafe fn window_set_title(handle: i32, title: Text) {
    with(handle, (), |p| {
        p.title = title.as_str().to_owned();
        let title = p.title.clone();
        p.commands.document_set_title(DOCUMENT, &title);
    });
}

pub unsafe fn window_set_size(handle: i32, width: i32, height: i32) {
    with(handle, (), |p| {
        p.commands
            .xw_agent_set_size(AGENT, &f64::from(width.max(0)), &f64::from(height.max(0)))
    });
}

pub unsafe fn window_set_min_size(handle: i32, width: i32, height: i32) {
    with(handle, (), |p| {
        p.commands
            .xw_agent_set_min_size(AGENT, &f64::from(width.max(0)), &f64::from(height.max(0)))
    });
}

pub unsafe fn window_set_max_size(handle: i32, width: i32, height: i32) {
    with(handle, (), |p| {
        p.commands
            .xw_agent_set_max_size(AGENT, &f64::from(width.max(0)), &f64::from(height.max(0)))
    });
}

pub unsafe fn window_set_position(_: i32, _: i32, _: i32) {}

pub unsafe fn window_set_resizable(_: i32, _: bool) {}

pub unsafe fn window_set_minimized(_: i32, _: bool) {}

pub unsafe fn window_set_maximized(_: i32, _: bool) {}

pub unsafe fn window_set_fullscreen(handle: i32, yes: bool) {
    with(handle, (), |p| {
        p.commands.xw_agent_set_fullscreen(AGENT, &yes)
    });
}

pub unsafe fn window_set_decorations(_: i32, _: bool) {}

pub unsafe fn window_set_visible(handle: i32, yes: bool) {
    with(handle, (), |p| {
        p.visible = yes;
        p.commands.xw_agent_set_visible(AGENT, &yes);
    });
}

pub unsafe fn window_set_window_level(_: i32, _: i32) {}

pub unsafe fn window_set_transparent(_: i32, _: bool) {}

pub unsafe fn window_set_blur(_: i32, _: bool) {}

pub unsafe fn window_set_content_protected(_: i32, _: bool) {}

pub unsafe fn window_set_scale_sizing(handle: i32, sizing: i32) {
    with(handle, (), |p| {
        if let Some(sizing) = ScaleSizing::from_native(sizing) {
            p.sizing = sizing;
        }
    });
}

/// Answered at the agent's next frame, with no command to send.
pub unsafe fn window_request_redraw(handle: i32) {
    with(handle, (), |_| BLOCK.redraw.store(1, SeqCst));
}

pub unsafe fn window_focus(handle: i32) {
    with(handle, (), |p| p.commands.xw_agent_focus(AGENT));
}

pub unsafe fn window_request_attention(_: i32, _: i32) {}

pub unsafe fn window_set_cursor_icon(handle: i32, icon: i32) {
    let Some(icon) = CursorIcon::from_native(icon) else {
        return;
    };
    with(handle, (), |p| {
        p.cursor = cursor_css(icon);
        p.set_cursor();
    });
}

/// Sent at once: the agent copies the pixels while it runs the command.
#[allow(unused_unsafe)]
pub unsafe fn window_set_cursor_image(
    handle: i32,
    rgba: Buffer,
    width: i32,
    height: i32,
    hot_x: i32,
    hot_y: i32,
) {
    let pixels: Box<[u8]> = unsafe { rgba.as_slice() }.into();
    let need = i64::from(width.max(0)) * i64::from(height.max(0)) * 4;
    if need == 0 || pixels.len() as i64 != need {
        return host::raise(
            ErrorKind::Type,
            "window: a cursor is width by height RGBA pixels",
        );
    }
    with(handle, (), |p| {
        let bytes = wire::Bytes {
            address: pixels.as_ptr() as usize as u32,
            len: pixels.len() as u32,
        };
        p.staged.push(pixels);
        p.commands.xw_agent_set_cursor_image(
            AGENT,
            &bytes,
            &(width as u32),
            &(height as u32),
            &(hot_x.max(0) as u32),
            &(hot_y.max(0) as u32),
        );
        p.flush();
    });
}

pub unsafe fn window_set_cursor_visible(handle: i32, yes: bool) {
    with(handle, (), |p| {
        p.cursor_visible = yes;
        p.set_cursor();
    });
}

/// A page locks the pointer, which needs a user's input to take; it cannot
/// confine it.
pub unsafe fn window_set_cursor_grab(handle: i32, grab: i32) -> bool {
    let lock = matches!(CursorGrab::from_native(grab), Some(CursorGrab::Locked));
    with(handle, false, |p| {
        p.commands.xw_agent_set_cursor_grab(AGENT, &lock);
        lock || CursorGrab::from_native(grab) == Some(CursorGrab::None)
    })
}

pub unsafe fn window_set_cursor_position(_: i32, _: f64, _: f64) -> bool {
    false
}

pub unsafe fn window_set_ime_allowed(handle: i32, yes: bool) {
    with(handle, (), |p| {
        p.commands.xw_agent_set_ime_allowed(AGENT, &yes)
    });
}

pub unsafe fn window_set_ime_cursor_area(handle: i32, x: f64, y: f64, width: f64, height: f64) {
    with(handle, (), |p| {
        p.commands
            .xw_agent_set_ime_area(AGENT, &x, &y, &width.max(0.0), &height.max(0.0))
    });
}

pub unsafe fn window_request_activation_token(_: i32) -> i64 {
    0
}

// -- monitors ---------------------------------------------------------------

pub unsafe fn window_current_monitor(handle: i32) -> i32 {
    with(handle, 0, |_| SCREEN)
}

pub unsafe fn window_primary_monitor(handle: i32) -> i32 {
    with(handle, 0, |_| SCREEN)
}

pub unsafe fn window_monitor_count(handle: i32) -> i32 {
    with(handle, 0, |_| 1)
}

pub unsafe fn window_monitor(handle: i32, index: i32) -> i32 {
    with(handle, 0, |_| if index == 0 { SCREEN } else { 0 })
}

pub unsafe fn monitor_valid(handle: i32) -> bool {
    handle == SCREEN
}

pub unsafe fn monitor_name(handle: i32) -> Text {
    let name = if handle == SCREEN {
        page().screen.2.clone()
    } else {
        String::new()
    };
    Text::new(&name)
}

/// The screen in physical pixels; the page reports it in CSS pixels.
fn screen(handle: i32, which: usize) -> i32 {
    if handle != SCREEN {
        return 0;
    }
    let p = page();
    let css = if which == 0 { p.screen.0 } else { p.screen.1 };
    (f64::from(css) * p.scale) as i32
}

pub unsafe fn monitor_width(handle: i32) -> i32 {
    screen(handle, 0)
}

pub unsafe fn monitor_height(handle: i32) -> i32 {
    screen(handle, 1)
}

pub unsafe fn monitor_x(_: i32) -> i32 {
    0
}

pub unsafe fn monitor_y(_: i32) -> i32 {
    0
}

pub unsafe fn monitor_scale_factor(handle: i32) -> f64 {
    if handle == SCREEN { page().scale } else { 1.0 }
}

pub unsafe fn monitor_refresh_rate(_: i32) -> i32 {
    0
}
