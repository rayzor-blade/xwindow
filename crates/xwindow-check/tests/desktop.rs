//! Opens real windows through the native backend and drives them with
//! synthetic input: xdotool on X11; on Wayland, a virtual pointer and
//! keyboard under sway or mutter's RemoteDesktop under GNOME; SendInput on
//! Windows. Each check prints PASS, FAIL or
//! SKIP, and any FAIL fails the run.
//!
//! It is a plain binary, not a libtest harness, because winit wants the main
//! thread.

use std::process::{Command, ExitCode};
use std::time::{Duration, Instant};

use xwindow_check::backend as native;
use xwindow_check::runtime::{Buffer, Text, host};
use xwindow_check::{
    CursorGrab, CursorIcon, Event, Key, KeyCode, KeyEvent, KeySupplement, Modifiers, MouseButton,
    MouseElementState, MouseScrollDelta, NamedKey, OptionalBytes, OptionalText, PhysicalKey,
    WindowAttributes,
};

/// XWINDOW_DESKTOP=1 opens real windows; unset, the test passes without a
/// display. Safe anywhere a display is free to take focus and input.
const GATE: &str = "XWINDOW_DESKTOP";

/// XWINDOW_DESKTOP_TRACE=1 prints every event as it arrives; safe, only noisy.
const TRACE: &str = "XWINDOW_DESKTOP_TRACE";

const PATIENCE: Duration = Duration::from_secs(5);
const QUIET: Duration = Duration::from_millis(400);

/// A 1x1 orange PNG.
const PNG: &[u8] = &[
    0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x48, 0x44, 0x52,
    0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x00, 0x01, 0x08, 0x06, 0x00, 0x00, 0x00, 0x1f, 0x15, 0xc4,
    0x89, 0x00, 0x00, 0x00, 0x0d, 0x49, 0x44, 0x41, 0x54, 0x78, 0x9c, 0x63, 0xf8, 0xdf, 0xc0, 0xf0,
    0x1f, 0x00, 0x06, 0x80, 0x02, 0x7f, 0x10, 0x4c, 0x1b, 0xe1, 0x00, 0x00, 0x00, 0x00, 0x49, 0x45,
    0x4e, 0x44, 0xae, 0x42, 0x60, 0x82,
];

const A: usize = 0;
const B: usize = 1;

fn main() -> ExitCode {
    if std::env::var_os(GATE).is_none_or(|v| v != "1") {
        println!("desktop: skipped; set {GATE}=1 with a display to open real windows");
        return ExitCode::SUCCESS;
    }
    let external = std::env::var_os("XWINDOW_EXTERNAL_PUMP").is_some_and(|v| v == "1");
    let mut desktop = Desktop { external, ..Default::default() };
    if external { native::external_pump(true).expect("external pump"); }
    desktop.run();
    if external { native::external_pump(false).expect("release external pump"); }
    desktop.report()
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
enum Platform {
    AppKit,
    Win32,
    X11,
    Wayland,
    #[default]
    Other,
}

impl Platform {
    fn of(code: i32) -> Self {
        match code {
            xwindow_core::raw::APPKIT => Self::AppKit,
            xwindow_core::raw::WIN32 => Self::Win32,
            xwindow_core::raw::XLIB => Self::X11,
            xwindow_core::raw::WAYLAND => Self::Wayland,
            _ => Self::Other,
        }
    }

    fn linux(self) -> bool {
        matches!(self, Self::X11 | Self::Wayland)
    }
}

struct Win {
    handle: i32,
    name: &'static str,
    title: &'static str,
    open: bool,
    log: Vec<Event>,
    /// Drawn once, at the size the window opens at. Redrawing on every
    /// Resized answers configures sway has already replaced, and the two
    /// chase each other.
    frame: Option<paint::Frame>,
    color: u32,
}

impl Win {
    fn keep(&mut self, event: Event) {
        if std::env::var_os(TRACE).is_some() {
            println!("  {}: {event:?}", self.name);
        }
        self.log.push(event);
    }
}

enum Outcome {
    Pass,
    Skip(String),
    Fail(String),
}

#[derive(Default)]
struct Desktop {
    external: bool,
    platform: Platform,
    wins: Vec<Win>,
    input: Option<Box<dyn Input>>,
    /// Why there is no input driver, when there is none.
    no_input: String,
    results: Vec<(String, Outcome)>,
}

/// A check's result: `Ok(None)` passes, `Ok(Some(why))` skips.
type Checked = Result<Option<String>, String>;

fn skip(reason: impl Into<String>) -> Checked {
    Ok(Some(reason.into()))
}

impl Desktop {
    fn run(&mut self) {
        self.check("open two windows", Self::open_two);
        if self.wins.len() < 2 {
            return;
        }
        (self.input, self.no_input) = match input_for(self.platform, &self.wins) {
            Ok(input) => (Some(input), String::new()),
            Err(why) => (None, why),
        };
        match &self.input {
            Some(input) => println!("desktop: {:?}, input via {}", self.platform, input.name()),
            None => println!("desktop: {:?}, no input: {}", self.platform, self.no_input),
        }
        self.pump(QUIET);
        self.check("windows report size, title and monitor", Self::describe);
        self.check("events go to the window they are for", Self::routed);
        self.check("Window.focus focuses the window", Self::backend_focus);
        self.check("keyboard input", Self::keyboard);
        self.check("pointer input", Self::pointer);
        self.check("wheel input", Self::wheel);
        self.check(
            "input goes to the window under it or focused",
            Self::input_routing,
        );
        self.check(
            "raw device events go to the focused window",
            Self::device_routing,
        );
        self.check("clipboard", Self::clipboard);
        self.check("redraws paced by presented frames", Self::paced_redraws);
        self.check("cursor icons", Self::cursor_icons);
        self.check("cursor image", Self::cursor_image);
        self.check("cursor position", Self::cursor_position);
        self.check("cursor grab", Self::cursor_grab);
        self.check("set and get position", Self::position);
        self.check("fullscreen", Self::fullscreen);
        self.check("activation token", Self::activation_token);
        self.check("non-UTF-8 dropped path", Self::non_utf8_path);
        self.check("closing one window leaves the other", Self::close_one);
        for w in &self.wins {
            if w.open {
                unsafe { native::window_close(w.handle) };
            }
        }
    }

    fn report(&self) -> ExitCode {
        let failed: Vec<&str> = self
            .results
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Fail(_)))
            .map(|(n, _)| n.as_str())
            .collect();
        let skipped = self
            .results
            .iter()
            .filter(|(_, o)| matches!(o, Outcome::Skip(_)))
            .count();
        println!(
            "desktop on {:?}: {} passed, {} skipped, {} failed",
            self.platform,
            self.results.len() - failed.len() - skipped,
            skipped,
            failed.len()
        );
        if failed.is_empty() {
            ExitCode::SUCCESS
        } else {
            println!("desktop failed: {}", failed.join("; "));
            ExitCode::FAILURE
        }
    }

    /// Runs one check, failing it as well when the backend raised in it
    /// without the check taking the raise.
    fn check(&mut self, name: &str, body: fn(&mut Self) -> Checked) {
        host::raised();
        let outcome = match body(self) {
            Ok(None) => Outcome::Pass,
            Ok(Some(reason)) => Outcome::Skip(reason),
            Err(why) => Outcome::Fail(why),
        };
        let raised = host::raised();
        let outcome = match outcome {
            Outcome::Fail(why) if !raised.is_empty() => {
                Outcome::Fail(format!("{why}; raised {raised:?}"))
            }
            _ if !raised.is_empty() => Outcome::Fail(format!("raised {raised:?}")),
            outcome => outcome,
        };
        match &outcome {
            Outcome::Pass => println!("PASS {name}"),
            Outcome::Skip(why) => println!("SKIP {name}: {why}"),
            Outcome::Fail(why) => println!("FAIL {name}: {why}"),
        }
        self.results.push((name.to_owned(), outcome));
    }

    // -- the event loop -------------------------------------------------------

    /// Pumps for `span`, keeping every window's events in its log.
    fn pump(&mut self, span: Duration) {
        let end = Instant::now() + span;
        loop {
            self.collect();
            let left = end.saturating_duration_since(Instant::now());
            if left.is_zero() {
                break;
            }
            if self.external {
                native::pump_external(Some(left.min(Duration::from_millis(25))));
                continue;
            }
            let Some(w) = self.wins.iter_mut().find(|w| w.open) else {
                std::thread::sleep(left);
                break;
            };
            let timeout = left.min(Duration::from_millis(25)).as_secs_f64();
            let event = unsafe { native::window_wait(w.handle, timeout) };
            if event != Event::None {
                w.keep(event);
            }
        }
    }

    fn collect(&mut self) {
        if self.external { native::pump_external(Some(Duration::ZERO)); }
        for w in self.wins.iter_mut().filter(|w| w.open) {
            loop {
                let event = unsafe { native::window_poll(w.handle) };
                if event == Event::None {
                    break;
                }
                w.keep(event);
            }
        }
    }

    fn marks(&self) -> Vec<usize> {
        self.wins.iter().map(|w| w.log.len()).collect()
    }

    /// The first event since `from` in `w`'s log that `matches`, pumping
    /// until it comes or patience runs out.
    fn expect(
        &mut self,
        w: usize,
        from: usize,
        what: &str,
        matches: impl Fn(&Event) -> bool,
    ) -> Result<Event, String> {
        let end = Instant::now() + PATIENCE;
        loop {
            if let Some(event) = self.wins[w].log[from..].iter().find(|e| matches(e)) {
                return Ok(event.clone());
            }
            if Instant::now() >= end {
                return Err(format!(
                    "{what} never reached window {}; it saw {}",
                    self.wins[w].name,
                    self.recent(w, from)
                ));
            }
            self.pump(Duration::from_millis(50));
        }
    }

    /// Fails when `w` has an event since `from` that `matches`.
    fn absent(
        &self,
        w: usize,
        from: usize,
        what: &str,
        matches: impl Fn(&Event) -> bool,
    ) -> Result<(), String> {
        match self.wins[w].log[from..].iter().find(|e| matches(e)) {
            Some(event) => Err(format!(
                "window {} got {what} meant for the other: {event:?}",
                self.wins[w].name
            )),
            None => Ok(()),
        }
    }

    fn recent(&self, w: usize, from: usize) -> String {
        let seen: Vec<String> = self.wins[w].log[from..]
            .iter()
            .filter(|e| !matches!(e, Event::RedrawRequested))
            .map(|e| format!("{e:?}"))
            .collect();
        if seen.is_empty() {
            return "nothing".into();
        }
        let skipped = seen.len().saturating_sub(12);
        let shown = seen[skipped..].join(", ");
        if skipped > 0 {
            format!("{skipped} events, then {shown}")
        } else {
            shown
        }
    }

    fn handle(&self, w: usize) -> i32 {
        self.wins[w].handle
    }

    fn scale(&self, w: usize) -> f64 {
        unsafe { native::window_scale_factor(self.handle(w)) }
    }

    /// Focuses `w` the way the platform allows: through the input driver
    /// where it focuses, otherwise through the backend.
    fn focus(&mut self, w: usize) -> Result<(), String> {
        let from = self.wins[w].log.len();
        let handle = self.handle(w);
        let win = &self.wins[w];
        match self.input.as_mut().and_then(|i| i.focus(win)) {
            Some(done) => done?,
            None => unsafe { native::window_focus(handle) },
        }
        if unsafe { native::window_has_focus(handle) } {
            self.pump(QUIET);
            return Ok(());
        }
        self.expect(w, from, "Focused(true)", |e| {
            matches!(e, Event::Focused { focused: true })
        })?;
        Ok(())
    }

    fn input(&mut self) -> Result<&mut dyn Input, String> {
        match self.input.as_deref_mut() {
            Some(input) => Ok(input),
            None => Err(self.no_input.clone()),
        }
    }

    /// Points at logical `(x, y)` in `w`'s client area.
    fn point(&mut self, w: usize, x: f64, y: f64) -> Result<(), String> {
        let win = &self.wins[w];
        match self.input.as_deref_mut() {
            Some(input) => input.point(win, x, y),
            None => Err(self.no_input.clone()),
        }
    }

    // -- checks -----------------------------------------------------------------

    fn open_two(&mut self) -> Checked {
        for (name, title, x, color) in [
            ("A", "xwindow-desktop-a", 40, 0x2060c0),
            ("B", "xwindow-desktop-b", 420, 0xc06020),
        ] {
            let mut a = WindowAttributes::new();
            WindowAttributes::title(&mut a, Text::new(title));
            WindowAttributes::width(&mut a, 320);
            WindowAttributes::height(&mut a, 240);
            WindowAttributes::x(&mut a, x);
            WindowAttributes::y(&mut a, 40);
            WindowAttributes::visible(&mut a, true);
            let handle = unsafe { native::window_open(&a) };
            if handle == 0 {
                return Err(format!("window {name} did not open: {:?}", host::raised()));
            }
            self.wins.push(Win {
                handle,
                name,
                title,
                open: true,
                log: Vec::new(),
                frame: None,
                color,
            });
        }
        let (a, b) = (self.handle(A), self.handle(B));
        if a == b {
            return Err(format!("both windows have handle {a}"));
        }
        if !unsafe { native::window_valid(a) && native::window_valid(b) } {
            return Err("a window opened but is not valid".into());
        }
        self.platform = Platform::of(unsafe { native::window_platform(a) });
        // Wayland maps a window only once it shows a frame.
        if self.platform == Platform::Wayland {
            for w in &mut self.wins {
                let mut frame = paint::Frame::new(w.handle)?;
                let (width, height) = unsafe {
                    (
                        native::window_width(w.handle),
                        native::window_height(w.handle),
                    )
                };
                frame.draw(width as u32, height as u32, w.color)?;
                w.frame = Some(frame);
            }
        }
        Ok(None)
    }

    fn describe(&mut self) -> Checked {
        for w in [A, B] {
            let h = self.handle(w);
            let (width, height) = unsafe { (native::window_width(h), native::window_height(h)) };
            if width <= 0 || height <= 0 {
                return Err(format!("window {} is {width}x{height}", self.wins[w].name));
            }
            let title = unsafe { native::window_title(h) };
            if title.as_str() != self.wins[w].title {
                return Err(format!(
                    "window {} reads back title {:?}, not {:?}",
                    self.wins[w].name,
                    title.as_str(),
                    self.wins[w].title
                ));
            }
        }
        let h = self.handle(A);
        if unsafe { native::window_monitor_count(h) } < 1 {
            return Err("no monitors".into());
        }
        let monitor = unsafe { native::window_current_monitor(h) };
        let (mw, mh) = unsafe {
            (
                native::monitor_width(monitor),
                native::monitor_height(monitor),
            )
        };
        if !unsafe { native::monitor_valid(monitor) } || mw <= 0 || mh <= 0 {
            return Err(format!("current monitor {monitor} is {mw}x{mh}"));
        }
        Ok(None)
    }

    fn routed(&mut self) -> Checked {
        let resized = |e: &Event| matches!(e, Event::Resized { .. });
        for (w, other, size) in [(B, A, (360, 260)), (A, B, (340, 250))] {
            let marks = self.marks();
            unsafe { native::window_set_size(self.handle(w), size.0, size.1) };
            let what = format!("Resized after {}'s setSize", self.wins[w].name);
            self.expect(w, marks[w], &what, resized)?;
            self.pump(QUIET);
            self.absent(other, marks[other], "a Resized", resized)?;
        }
        Ok(None)
    }

    fn backend_focus(&mut self) -> Checked {
        if self.platform == Platform::Wayland {
            return skip("winit cannot focus a window on Wayland; the compositor does");
        }
        // Starting with the window that lacks focus, so each focus is a change.
        let order = if unsafe { native::window_has_focus(self.handle(B)) } {
            [(A, B), (B, A)]
        } else {
            [(B, A), (A, B)]
        };
        for (w, other) in order {
            let marks = self.marks();
            unsafe { native::window_focus(self.handle(w)) };
            self.expect(w, marks[w], "Focused(true)", |e| {
                matches!(e, Event::Focused { focused: true })
            })?;
            if !unsafe { native::window_has_focus(self.handle(w)) } {
                return Err(format!(
                    "window {} has no focus after Focused(true)",
                    self.wins[w].name
                ));
            }
            self.expect(other, marks[other], "Focused(false)", |e| {
                matches!(e, Event::Focused { focused: false })
            })?;
        }
        Ok(None)
    }

    fn keyboard(&mut self) -> Checked {
        self.input()?;
        self.focus(A)?;

        let marks = self.marks();
        self.input()?.key(Stroke::A)?;
        let press = self.expect(A, marks[A], "KeyboardInput a pressed", |e| {
            pressed_key(e).is_some_and(|k| k.logical_key == Key::Character { text: "a".into() })
        })?;
        let k = pressed_key(&press).expect("matched above");
        if k.text != (OptionalText::Some { text: "a".into() }) {
            return Err(format!("a carries text {:?}", k.text));
        }
        if k.physical_key
            != (PhysicalKey::Code {
                code: KeyCode::KeyA,
            })
        {
            return Err(format!("a has physical key {:?}", k.physical_key));
        }
        self.expect(A, marks[A], "KeyboardInput a released", |e| {
            released_key(e).is_some_and(|k| k.logical_key == Key::Character { text: "a".into() })
        })?;

        let marks = self.marks();
        self.input()?.key(Stroke::ShiftB)?;
        self.expect(A, marks[A], "ModifiersChanged with shift", |e| {
            matches!(
                e,
                Event::ModifiersChanged {
                    modifiers: Modifiers::State { shift: true, .. }
                }
            )
        })?;
        let press = self.expect(A, marks[A], "KeyboardInput B pressed", |e| {
            pressed_key(e).is_some_and(|k| k.logical_key == Key::Character { text: "B".into() })
        })?;
        let k = pressed_key(&press).expect("matched above");
        if k.text != (OptionalText::Some { text: "B".into() }) {
            return Err(format!("Shift-B carries text {:?}", k.text));
        }
        if k.physical_key
            != (PhysicalKey::Code {
                code: KeyCode::KeyB,
            })
        {
            return Err(format!("Shift-B has physical key {:?}", k.physical_key));
        }
        match &k.supplement {
            KeySupplement::Supplement {
                key_without_modifiers: Key::Character { text },
                ..
            } if text == "b" => {}
            other => return Err(format!("Shift-B has supplement {other:?}")),
        }

        let marks = self.marks();
        self.input()?.key(Stroke::Left)?;
        self.expect(A, marks[A], "KeyboardInput ArrowLeft pressed", |e| {
            pressed_key(e).is_some_and(|k| {
                k.logical_key
                    == (Key::Named {
                        key: NamedKey::ArrowLeft,
                    })
                    && k.physical_key
                        == (PhysicalKey::Code {
                            code: KeyCode::ArrowLeft,
                        })
            })
        })?;
        Ok(None)
    }

    fn pointer(&mut self) -> Checked {
        self.input()?;
        self.focus(A)?;
        let scale = self.scale(A);
        let marks = self.marks();
        self.point(A, 60.0, 50.0)?;
        let (x, y) = (60.0 * scale, 50.0 * scale);
        self.expect(A, marks[A], "CursorMoved to (60, 50)", |e| {
            matches!(e, Event::CursorMoved { x: ex, y: ey, .. }
                if (ex - x).abs() <= 2.0 * scale && (ey - y).abs() <= 2.0 * scale)
        })?;
        let marks = self.marks();
        self.input()?.click()?;
        self.expect(A, marks[A], "left MouseInput pressed", |e| {
            is_click(e, MouseElementState::Pressed)
        })?;
        self.expect(A, marks[A], "left MouseInput released", |e| {
            is_click(e, MouseElementState::Released)
        })?;
        Ok(None)
    }

    fn wheel(&mut self) -> Checked {
        self.input()?;
        let marks = self.marks();
        self.point(A, 70.0, 60.0)?;
        self.input()?.scroll_up()?;
        let event = self.expect(A, marks[A], "MouseWheel", |e| {
            matches!(e, Event::MouseWheel { .. })
        })?;
        let Event::MouseWheel { delta, .. } = event else {
            unreachable!()
        };
        match delta {
            MouseScrollDelta::LineDelta { y, .. } | MouseScrollDelta::PixelDelta { y, .. }
                if y > 0.0 =>
            {
                Ok(None)
            }
            delta => Err(format!("scrolling up gave {delta:?}; up is positive y")),
        }
    }

    fn input_routing(&mut self) -> Checked {
        self.input()?;
        // The pointer: over B, B gets the click and A does not.
        self.focus(B)?;
        let marks = self.marks();
        self.point(B, 80.0, 70.0)?;
        self.input()?.click()?;
        self.expect(B, marks[B], "left MouseInput released over B", |e| {
            is_click(e, MouseElementState::Released)
        })?;
        self.pump(QUIET);
        self.absent(A, marks[A], "a MouseInput", |e| {
            matches!(e, Event::MouseInput { .. })
        })?;
        self.absent(A, marks[A], "a CursorMoved", |e| {
            matches!(e, Event::CursorMoved { .. })
        })?;

        // The keyboard: B focused, B gets the key and A does not.
        let marks = self.marks();
        self.input()?.key(Stroke::A)?;
        self.expect(B, marks[B], "KeyboardInput while B is focused", |e| {
            pressed_key(e).is_some()
        })?;
        self.pump(QUIET);
        self.absent(A, marks[A], "a KeyboardInput", |e| {
            matches!(e, Event::KeyboardInput { .. })
        })?;
        Ok(None)
    }

    fn device_routing(&mut self) -> Checked {
        self.input()?;
        let is_device = |e: &Event| matches!(e, Event::Device { .. });
        for (w, other) in [(B, A), (A, B)] {
            self.focus(w)?;
            // The click lands on the focused window, so it moves no focus.
            self.point(w, 100.0, 100.0)?;
            self.pump(QUIET);
            let marks = self.marks();
            self.input()?.nudge()?;
            self.input()?.key(Stroke::A)?;
            self.input()?.click()?;
            self.expect(w, marks[w], "a Device event", is_device)?;
            self.pump(QUIET);
            self.absent(other, marks[other], "a Device event", is_device)?;
        }
        Ok(None)
    }

    /// After the input checks: Wayland takes a selection only from a client
    /// that has had input. Text, then typed data written together.
    fn clipboard(&mut self) -> Checked {
        let text = "xwindow clipboard \u{2713} \u{fc}";
        unsafe { native::window_set_clipboard_text(Text::new(text)) };
        self.pump(QUIET);
        match unsafe { native::window_clipboard_text() } {
            OptionalText::Some { text: got } if got == text => {}
            got => return Err(format!("set {text:?}, read back {got:?}")),
        }

        let items: [(&str, &[u8]); 3] = [
            ("text/plain", "xwindow typed \u{2713}".as_bytes()),
            ("text/html", b"<b>xwindow</b> typed"),
            ("image/png", PNG),
        ];
        let handle = unsafe { native::clipboard_items_create() };
        for (mime, bytes) in items {
            unsafe { native::clipboard_items_add(handle, Text::new(mime), Buffer::new(bytes)) };
        }
        if !unsafe { native::clipboard_items_write(handle) } {
            return Err("the platform refused the typed items".into());
        }
        self.pump(QUIET);
        let types: Vec<String> = (0..unsafe { native::window_clipboard_type_count() })
            .map(|i| {
                unsafe { native::window_clipboard_type(i) }
                    .as_str()
                    .to_owned()
            })
            .collect();
        for (mime, bytes) in items {
            if !types.iter().any(|t| t == mime) {
                return Err(format!(
                    "{mime} is not among the clipboard's types {types:?}"
                ));
            }
            match unsafe { native::window_clipboard_data(Text::new(mime)) } {
                OptionalBytes::Some { bytes: got } if got.0 == bytes => {}
                OptionalBytes::Some { bytes: got } => {
                    return Err(format!(
                        "{mime} read back {} bytes, not {}",
                        got.0.len(),
                        bytes.len()
                    ));
                }
                OptionalBytes::None => return Err(format!("{mime} read back none")),
            }
        }
        Ok(None)
    }

    /// Each RedrawRequested after a frame presented behind prePresentNotify:
    /// on Wayland it waits for that frame's callback, and must still come.
    fn paced_redraws(&mut self) -> Checked {
        let h = self.handle(A);
        for _ in 0..3 {
            let marks = self.marks();
            unsafe { native::window_pre_present_notify(h) };
            let (width, height) = unsafe { (native::window_width(h), native::window_height(h)) };
            let color = self.wins[A].color;
            if let Some(frame) = self.wins[A].frame.as_mut() {
                frame.draw(width as u32, height as u32, color)?;
            }
            unsafe { native::window_request_redraw(h) };
            self.expect(
                A,
                marks[A],
                "RedrawRequested after a presented frame",
                |e| matches!(e, Event::RedrawRequested),
            )?;
        }
        Ok(None)
    }

    fn cursor_icons(&mut self) -> Checked {
        let h = self.handle(A);
        let mut count = 0;
        while let Some(icon) = CursorIcon::from_native(count) {
            unsafe { native::window_set_cursor_icon(h, icon.native()) };
            self.pump(Duration::from_millis(5));
            count += 1;
        }
        unsafe { native::window_set_cursor_icon(h, CursorIcon::Default.native()) };
        unsafe { native::window_set_cursor_visible(h, false) };
        self.pump(Duration::from_millis(20));
        unsafe { native::window_set_cursor_visible(h, true) };
        if count < 30 {
            return Err(format!("only {count} cursor icons"));
        }
        Ok(None)
    }

    fn cursor_image(&mut self) -> Checked {
        let h = self.handle(A);
        let pixels: Vec<u8> = (0..16 * 16)
            .flat_map(|i| [i as u8, 0x80, 0xff - i as u8, 0xff])
            .collect();
        unsafe { native::window_set_cursor_image(h, Buffer::new(&pixels), 16, 16, 8, 8) };
        self.pump(QUIET);
        let raised = host::raised();
        if !raised.is_empty() {
            return Err(format!("a 16x16 cursor raised {raised:?}"));
        }
        // Too few bytes for its size is the caller's mistake, raised as such.
        unsafe { native::window_set_cursor_image(h, Buffer::new(&pixels[..10]), 16, 16, 0, 0) };
        let raised = host::raised();
        if !raised.iter().any(|r| r.starts_with("Type")) {
            return Err(format!(
                "a short cursor buffer raised {raised:?}, not a type error"
            ));
        }
        unsafe { native::window_set_cursor_icon(h, CursorIcon::Default.native()) };
        Ok(None)
    }

    fn cursor_position(&mut self) -> Checked {
        if self.platform == Platform::Wayland {
            // Checked under "cursor grab", while locked.
            return skip("Wayland moves the cursor only while it is locked");
        }
        self.focus(A)?;
        let scale = self.scale(A);
        let marks = self.marks();
        if !unsafe { native::window_set_cursor_position(self.handle(A), 100.0, 80.0) } {
            return Err("setCursorPosition returned false".into());
        }
        let (x, y) = (100.0 * scale, 80.0 * scale);
        self.expect(A, marks[A], "CursorMoved to (100, 80)", |e| {
            matches!(e, Event::CursorMoved { x: ex, y: ey, .. }
                if (ex - x).abs() <= 1.0 && (ey - y).abs() <= 1.0)
        })?;
        Ok(None)
    }

    fn cursor_grab(&mut self) -> Checked {
        self.focus(A)?;
        if self.input.is_some() {
            self.point(A, 90.0, 90.0)?;
        }
        self.pump(QUIET);
        let h = self.handle(A);
        let grab = |mode: CursorGrab| unsafe { native::window_set_cursor_grab(h, mode.native()) };
        // What winit supports where.
        let (confined, locked) = match self.platform {
            Platform::X11 => (true, false),
            Platform::AppKit => (false, true),
            Platform::Win32 | Platform::Wayland | Platform::Other => (true, true),
        };
        let mut wrong = Vec::new();
        if grab(CursorGrab::Confined) != confined {
            wrong.push(format!("Confined gave {}", !confined));
        }
        self.pump(Duration::from_millis(100));
        if !grab(CursorGrab::None) {
            wrong.push("None after Confined gave false".into());
        }
        if grab(CursorGrab::Locked) != locked {
            wrong.push(format!("Locked gave {}", !locked));
        }
        self.pump(Duration::from_millis(100));
        if self.platform == Platform::Wayland
            && locked
            && !unsafe { native::window_set_cursor_position(h, 50.0, 50.0) }
        {
            wrong.push("setCursorPosition while locked gave false".into());
        }
        if !grab(CursorGrab::None) {
            wrong.push("None after Locked gave false".into());
        }
        self.pump(QUIET);
        if wrong.is_empty() {
            Ok(None)
        } else {
            Err(wrong.join("; "))
        }
    }

    fn position(&mut self) -> Checked {
        if self.platform == Platform::Wayland {
            return skip("Wayland does not let a client place its window or read its position");
        }
        let h = self.handle(A);
        let scale = self.scale(A);
        let marks = self.marks();
        unsafe { native::window_set_position(h, 150, 120) };
        let want = (
            (150.0 * scale).round() as i32,
            (120.0 * scale).round() as i32,
        );
        self.expect(A, marks[A], "Moved", |e| matches!(e, Event::Moved { .. }))?;
        let end = Instant::now() + PATIENCE;
        loop {
            let got = unsafe { (native::window_x(h), native::window_y(h)) };
            if got == want {
                return Ok(None);
            }
            if Instant::now() >= end {
                return Err(format!("asked for {want:?}, the window is at {got:?}"));
            }
            self.pump(Duration::from_millis(50));
        }
    }

    fn fullscreen(&mut self) -> Checked {
        let h = self.handle(A);
        self.focus(A)?;
        self.pump(QUIET);
        let before = unsafe { (native::window_width(h), native::window_height(h)) };
        let monitor = unsafe { native::window_current_monitor(h) };
        let screen = unsafe {
            (
                native::monitor_width(monitor),
                native::monitor_height(monitor),
            )
        };
        let sized = |(w, h): (i32, i32)| {
            move |e: &Event| {
                matches!(e, Event::Resized { width, height }
                    if *width == i64::from(w) && *height == i64::from(h))
            }
        };
        let marks = self.marks();
        unsafe { native::window_set_fullscreen(h, true) };
        let what = format!("Resized to the monitor's {screen:?}");
        if self.platform == Platform::AppKit {
            // AppKit keeps fullscreen content below a camera housing.
            self.expect(A, marks[A], &what, |e| {
                matches!(e, Event::Resized { width, height }
                    if *width == i64::from(screen.0)
                        && *height <= i64::from(screen.1)
                        && *height > i64::from(screen.1) * 9 / 10)
            })?;
        } else {
            self.expect(A, marks[A], &what, sized(screen))?;
        }
        if !unsafe { native::window_is_fullscreen(h) } {
            return Err("isFullscreen is false while fullscreen".into());
        }
        if self.platform == Platform::AppKit {
            // Leaving while AppKit still animates into the fullscreen space
            // leaves the window at the fullscreen size.
            self.pump(Duration::from_secs(1));
        }
        let marks = self.marks();
        unsafe { native::window_set_fullscreen(h, false) };
        self.expect(
            A,
            marks[A],
            &format!("Resized back to {before:?}"),
            sized(before),
        )?;
        if unsafe { native::window_is_fullscreen(h) } {
            return Err("isFullscreen is true after leaving fullscreen".into());
        }
        Ok(None)
    }

    fn activation_token(&mut self) -> Checked {
        let h = self.handle(A);
        let from = self.wins[A].log.len();
        let serial = unsafe { native::window_request_activation_token(h) };
        if !self.platform.linux() {
            return match serial {
                0 => skip("activation tokens are X11 and Wayland only; the request returns 0"),
                s => Err(format!("an unsupported request returned serial {s}")),
            };
        }
        if serial <= 0 {
            return Err(format!("requestActivationToken returned {serial}"));
        }
        let done = self.expect(
            A,
            from,
            "ActivationTokenDone",
            |e| matches!(e, Event::ActivationTokenDone { serial: s, .. } if *s == serial),
        )?;
        match done {
            Event::ActivationTokenDone { token, .. } if !token.is_empty() => Ok(None),
            other => Err(format!("the token is empty: {other:?}")),
        }
    }

    fn non_utf8_path(&mut self) -> Checked {
        skip(match self.platform {
            Platform::X11 => {
                "winit's X11 drop handler rejects URIs that are not UTF-8 before making an event"
            }
            Platform::Wayland => "winit has no drag and drop on Wayland",
            _ => "a drop cannot be synthesized here",
        })
    }

    fn close_one(&mut self) -> Checked {
        let (a, b) = (self.handle(A), self.handle(B));
        unsafe { native::window_close(b) };
        self.wins[B].open = false;
        if unsafe { native::window_valid(b) } {
            return Err("B is valid after close".into());
        }
        if unsafe { native::window_poll(b) } != Event::None {
            return Err("polling closed B gave an event".into());
        }
        if !unsafe { native::window_valid(a) } {
            return Err("A is invalid after B closed".into());
        }
        let marks = self.marks();
        unsafe { native::window_set_size(a, 300, 220) };
        self.expect(A, marks[A], "Resized after A's setSize", |e| {
            matches!(e, Event::Resized { .. })
        })?;
        Ok(None)
    }
}

struct KeyInput {
    physical_key: PhysicalKey,
    logical_key: Key,
    text: OptionalText,
    supplement: KeySupplement,
}

fn pressed_key(event: &Event) -> Option<KeyInput> {
    key_in(event, MouseElementState::Pressed)
}

fn released_key(event: &Event) -> Option<KeyInput> {
    key_in(event, MouseElementState::Released)
}

fn key_in(event: &Event, want: MouseElementState) -> Option<KeyInput> {
    match event {
        Event::KeyboardInput {
            event:
                KeyEvent::Input {
                    physical_key,
                    logical_key,
                    text,
                    state,
                    supplement,
                    ..
                },
            ..
        } if *state == want => Some(KeyInput {
            physical_key: physical_key.clone(),
            logical_key: logical_key.clone(),
            text: text.clone(),
            supplement: supplement.clone(),
        }),
        _ => None,
    }
}

fn is_click(event: &Event, want: MouseElementState) -> bool {
    matches!(event, Event::MouseInput { state, button: MouseButton::Left, .. } if *state == want)
}

// -- frames ---------------------------------------------------------------------

#[cfg(target_os = "linux")]
mod paint {
    use std::num::NonZeroU32;

    use raw_window_handle::{
        DisplayHandle, HandleError, HasDisplayHandle, HasWindowHandle, RawDisplayHandle,
        RawWindowHandle, WindowHandle,
    };

    use super::native;

    /// A window's handles as the backend hands them out.
    #[derive(Clone, Copy)]
    struct Raw(RawDisplayHandle, RawWindowHandle);

    impl HasDisplayHandle for Raw {
        fn display_handle(&self) -> Result<DisplayHandle<'_>, HandleError> {
            Ok(unsafe { DisplayHandle::borrow_raw(self.0) })
        }
    }

    impl HasWindowHandle for Raw {
        fn window_handle(&self) -> Result<WindowHandle<'_>, HandleError> {
            Ok(unsafe { WindowHandle::borrow_raw(self.1) })
        }
    }

    /// A solid frame drawn on the CPU. The surface is declared first so it
    /// drops before its context.
    pub struct Frame {
        surface: softbuffer::Surface<Raw, Raw>,
        _context: softbuffer::Context<Raw>,
    }

    impl Frame {
        pub fn new(handle: i32) -> Result<Self, String> {
            let platform = unsafe { native::window_platform(handle) };
            let raw = [0, 1, 2, 3].map(|i| unsafe { native::window_raw(handle, i) });
            let (display, window) = unsafe { xwindow_core::raw::join(platform, raw) }
                .ok_or("the window has no handles")?;
            let raw = Raw(display, window);
            let context = softbuffer::Context::new(raw).map_err(|e| format!("softbuffer: {e}"))?;
            let surface =
                softbuffer::Surface::new(&context, raw).map_err(|e| format!("softbuffer: {e}"))?;
            Ok(Self {
                surface,
                _context: context,
            })
        }

        pub fn draw(&mut self, width: u32, height: u32, color: u32) -> Result<(), String> {
            let (Some(width), Some(height)) = (NonZeroU32::new(width), NonZeroU32::new(height))
            else {
                return Ok(());
            };
            let fail = |e: softbuffer::SoftBufferError| format!("softbuffer: {e}");
            self.surface.resize(width, height).map_err(fail)?;
            let mut buffer = self.surface.buffer_mut().map_err(fail)?;
            buffer.fill(color);
            buffer.present().map_err(fail)
        }
    }
}

/// Only Wayland needs a frame, and it is Linux only.
#[cfg(not(target_os = "linux"))]
mod paint {
    pub struct Frame;

    impl Frame {
        pub fn new(_: i32) -> Result<Self, String> {
            Err("frames are drawn only on Linux".into())
        }

        pub fn draw(&mut self, _: u32, _: u32, _: u32) -> Result<(), String> {
            Ok(())
        }
    }
}

// -- synthetic input ------------------------------------------------------------

#[derive(Clone, Copy)]
enum Stroke {
    A,
    ShiftB,
    Left,
}

trait Input {
    fn name(&self) -> &'static str;
    /// Focuses `w` when the driver is how the platform focuses, or `None`.
    fn focus(&mut self, _w: &Win) -> Option<Result<(), String>> {
        None
    }
    /// Moves the pointer to logical `(x, y)` in `w`'s client area.
    fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String>;
    /// Moves the pointer a little, relatively, as a device does.
    fn nudge(&mut self) -> Result<(), String>;
    fn click(&mut self) -> Result<(), String>;
    fn scroll_up(&mut self) -> Result<(), String>;
    fn key(&mut self, stroke: Stroke) -> Result<(), String>;
}

#[cfg_attr(not(target_os = "linux"), allow(unused_variables))]
fn input_for(platform: Platform, wins: &[Win]) -> Result<Box<dyn Input>, String> {
    match platform {
        Platform::X11 => {
            run("xdotool", &["version"]).map_err(|e| format!("no xdotool: {e}"))?;
            Ok(Box::new(Xdotool))
        }
        #[cfg(target_os = "linux")]
        Platform::Wayland if std::env::var_os("SWAYSOCK").is_some() => {
            Sway::new(wins).map(|s| Box::new(s) as Box<dyn Input>)
        }
        #[cfg(target_os = "linux")]
        Platform::Wayland => gnome::Mutter::new(wins)
            .map(|m| Box::new(m) as Box<dyn Input>)
            .map_err(|why| format!("not under sway (no SWAYSOCK), and not under GNOME: {why}")),
        #[cfg(windows)]
        Platform::Win32 => Ok(Box::new(windows_input::SendInput)),
        #[cfg(target_os = "macos")]
        Platform::AppKit => Ok(Box::new(quartz::Quartz)),
        _ => Err("no synthetic input for this platform".into()),
    }
}

fn run(program: &str, args: &[&str]) -> Result<(), String> {
    let out = Command::new(program)
        .args(args)
        .output()
        .map_err(|e| format!("{program}: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        let said = format!(
            "{} {}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        let said: String = said.split_whitespace().collect::<Vec<_>>().join(" ");
        let said: String = said.chars().take(240).collect();
        Err(format!("{program} {args:?}: {said}"))
    }
}

/// X11 through XTEST, which the server treats as a real device.
struct Xdotool;

impl Input for Xdotool {
    fn name(&self) -> &'static str {
        "xdotool"
    }

    fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String> {
        let xid = unsafe { native::window_raw(w.handle, 0) }.to_string();
        let scale = unsafe { native::window_scale_factor(w.handle) };
        let (x, y) = (
            (x * scale).round().to_string(),
            (y * scale).round().to_string(),
        );
        run(
            "xdotool",
            &["mousemove", "--sync", "--window", &xid, &x, &y],
        )
    }

    fn nudge(&mut self) -> Result<(), String> {
        run("xdotool", &["mousemove_relative", "--", "3", "2"])
    }

    fn click(&mut self) -> Result<(), String> {
        run("xdotool", &["click", "1"])
    }

    fn scroll_up(&mut self) -> Result<(), String> {
        run("xdotool", &["click", "4"])
    }

    fn key(&mut self, stroke: Stroke) -> Result<(), String> {
        let key = match stroke {
            Stroke::A => "a",
            Stroke::ShiftB => "shift+b",
            Stroke::Left => "Left",
        };
        run("xdotool", &["key", key])
    }
}

/// Wayland under sway: the test's own virtual pointer and keyboard give the
/// input, and sway places and focuses the windows. A wlroots seat has no
/// pointer or keyboard without a device, so the devices are held throughout.
#[cfg(target_os = "linux")]
struct Sway {
    seat: virtual_seat::Seat,
    origins: Vec<(&'static str, i32, i32)>,
}

#[cfg(target_os = "linux")]
impl Sway {
    fn new(wins: &[Win]) -> Result<Self, String> {
        // The one output; a window not yet shown on it has no current monitor.
        let monitor = unsafe { native::window_monitor(wins[0].handle, 0) };
        let extent = unsafe {
            (
                native::monitor_width(monitor),
                native::monitor_height(monitor),
            )
        };
        if extent.0 <= 0 || extent.1 <= 0 {
            return Err(format!("the output measures {extent:?}"));
        }
        let seat = virtual_seat::Seat::new(extent)?;
        let mut origins = Vec::new();
        for (i, w) in wins.iter().enumerate() {
            let (x, y) = (40 + 420 * i as i32, 40);
            let command = format!(
                "[title=\"^{}$\"] floating enable, border none, move absolute position {x} {y}",
                w.title
            );
            run("swaymsg", &[&command])?;
            origins.push((w.title, x, y));
        }
        Ok(Self { seat, origins })
    }
}

#[cfg(target_os = "linux")]
impl Input for Sway {
    fn name(&self) -> &'static str {
        "a virtual pointer and keyboard under sway"
    }

    fn focus(&mut self, w: &Win) -> Option<Result<(), String>> {
        Some(run(
            "swaymsg",
            &[&format!("[title=\"^{}$\"] focus", w.title)],
        ))
    }

    fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String> {
        let &(_, ox, oy) = self
            .origins
            .iter()
            .find(|(title, ..)| *title == w.title)
            .ok_or("an unarranged window")?;
        self.seat.point(ox as f64 + x, oy as f64 + y)
    }

    fn nudge(&mut self) -> Result<(), String> {
        self.seat.nudge(3.0, 2.0)
    }

    fn click(&mut self) -> Result<(), String> {
        self.seat.click()
    }

    fn scroll_up(&mut self) -> Result<(), String> {
        self.seat.scroll_up()
    }

    fn key(&mut self, stroke: Stroke) -> Result<(), String> {
        use virtual_seat::{KEY_A, KEY_B, KEY_LEFT, KEY_LEFTSHIFT, SHIFT};
        match stroke {
            Stroke::A => self.seat.keys(&[(KEY_A, 0)]),
            Stroke::ShiftB => self.seat.keys(&[(KEY_LEFTSHIFT, SHIFT), (KEY_B, SHIFT)]),
            Stroke::Left => self.seat.keys(&[(KEY_LEFT, 0)]),
        }
    }
}

/// Wayland under GNOME: mutter's RemoteDesktop gives the input, over a
/// screen cast of the monitor its absolute pointer motion is placed on. A
/// Wayland client cannot read where its window is, so each window's place is
/// found by sweeping the pointer until the window reports it. GNOME focuses a
/// window when it is clicked.
#[cfg(target_os = "linux")]
mod gnome {
    use std::collections::HashMap;
    use std::time::{Duration, Instant};

    use zbus::blocking::{Connection, Proxy};
    use zbus::zvariant::{OwnedObjectPath, Value};

    use super::{Event, Input, Stroke, Win, native};

    // evdev codes.
    const KEY_A: u32 = 30;
    const KEY_B: u32 = 48;
    const KEY_LEFTSHIFT: u32 = 42;
    const KEY_LEFT: u32 = 105;
    const BTN_LEFT: i32 = 0x110;
    /// Sweep spacing; smaller than either window.
    const STEP: usize = 60;

    pub struct Mutter {
        // The sessions live as long as the connection that asked for them.
        _connection: Connection,
        session: Proxy<'static>,
        stream: OwnedObjectPath,
        /// Each window's client area on the monitor, by title.
        origins: Vec<(&'static str, f64, f64)>,
    }

    fn fail(what: &str) -> impl Fn(zbus::Error) -> String + '_ {
        move |e| format!("{what}: {e}")
    }

    impl Mutter {
        pub fn new(wins: &[Win]) -> Result<Self, String> {
            let connection = Connection::session().map_err(fail("the session bus"))?;
            let remote = Proxy::new(
                &connection,
                "org.gnome.Mutter.RemoteDesktop",
                "/org/gnome/Mutter/RemoteDesktop",
                "org.gnome.Mutter.RemoteDesktop",
            )
            .map_err(fail("RemoteDesktop"))?;
            let path: OwnedObjectPath = remote
                .call("CreateSession", &())
                .map_err(fail("RemoteDesktop.CreateSession"))?;
            let session = Proxy::new(
                &connection,
                "org.gnome.Mutter.RemoteDesktop",
                path,
                "org.gnome.Mutter.RemoteDesktop.Session",
            )
            .map_err(fail("the remote desktop session"))?;
            let id: String = session
                .get_property("SessionId")
                .map_err(fail("SessionId"))?;
            let cast = Proxy::new(
                &connection,
                "org.gnome.Mutter.ScreenCast",
                "/org/gnome/Mutter/ScreenCast",
                "org.gnome.Mutter.ScreenCast",
            )
            .map_err(fail("ScreenCast"))?;
            let linked = HashMap::from([("remote-desktop-session-id", Value::from(id))]);
            let path: OwnedObjectPath = cast
                .call("CreateSession", &(linked,))
                .map_err(fail("ScreenCast.CreateSession"))?;
            let cast_session = Proxy::new(
                &connection,
                "org.gnome.Mutter.ScreenCast",
                path,
                "org.gnome.Mutter.ScreenCast.Session",
            )
            .map_err(fail("the screen cast session"))?;
            // The primary monitor.
            let stream: OwnedObjectPath = cast_session
                .call("RecordMonitor", &("", HashMap::<&str, Value>::new()))
                .map_err(fail("RecordMonitor"))?;
            let () = session.call("Start", &()).map_err(fail("Start"))?;
            let mut mutter = Self {
                _connection: connection,
                session,
                stream,
                origins: Vec::new(),
            };
            // The seat has a keyboard, which focus is, only once one types.
            mutter.key_strokes(&[KEY_LEFTSHIFT])?;
            mutter.find(wins)?;
            mutter.separate(wins)?;
            for (title, x, y) in &mutter.origins {
                println!("desktop: {title} found at {x}, {y}");
            }
            Ok(mutter)
        }

        fn to(&self, x: f64, y: f64) -> Result<(), String> {
            self.session
                .call::<_, _, ()>("NotifyPointerMotionAbsolute", &(self.stream.as_str(), x, y))
                .map_err(fail("NotifyPointerMotionAbsolute"))
        }

        /// Sweeps the monitor until every window has reported the pointer.
        fn find(&mut self, wins: &[Win]) -> Result<(), String> {
            let monitor = unsafe { native::window_monitor(wins[0].handle, 0) };
            let (width, height) = unsafe {
                (
                    native::monitor_width(monitor),
                    native::monitor_height(monitor),
                )
            };
            if width <= 0 || height <= 0 {
                return Err(format!("the monitor measures {width}x{height}"));
            }
            for y in (STEP / 2..height as usize).step_by(STEP) {
                for x in (STEP / 2..width as usize).step_by(STEP) {
                    self.to(x as f64, y as f64)?;
                    let end = Instant::now() + Duration::from_millis(15);
                    while Instant::now() < end {
                        for w in wins {
                            let event = unsafe { native::window_wait(w.handle, 0.005) };
                            if let Event::CursorMoved { x: cx, y: cy, .. } = event {
                                if !self.origins.iter().any(|(t, ..)| *t == w.title) {
                                    let scale = unsafe { native::window_scale_factor(w.handle) };
                                    self.origins.push((
                                        w.title,
                                        x as f64 - cx / scale,
                                        y as f64 - cy / scale,
                                    ));
                                }
                            }
                        }
                    }
                    if self.origins.len() == wins.len() {
                        return Ok(());
                    }
                }
            }
            Err(format!(
                "the pointer found {} of {} windows on the monitor",
                self.origins.len(),
                wins.len()
            ))
        }

        /// A window's client area on the monitor: origin and logical size.
        fn rect(&self, w: &Win) -> Result<(f64, f64, f64, f64), String> {
            let &(_, x, y) = self
                .origins
                .iter()
                .find(|(title, ..)| *title == w.title)
                .ok_or("an unfound window")?;
            let scale = unsafe { native::window_scale_factor(w.handle) };
            let (width, height) = unsafe {
                (
                    native::window_width(w.handle),
                    native::window_height(w.handle),
                )
            };
            Ok((x, y, f64::from(width) / scale, f64::from(height) / scale))
        }

        /// GNOME cascades new windows, so the second overlaps the first;
        /// the second is dragged clear of it with `dragWindow`, which is how
        /// a Wayland client moves itself.
        fn separate(&mut self, wins: &[Win]) -> Result<(), String> {
            const GAP: f64 = 60.0;
            let (ax, ay, aw, ah) = self.rect(&wins[0])?;
            let (bx, by, bw, bh) = self.rect(&wins[1])?;
            let overlap = |x: f64, y: f64| {
                x < ax + aw + GAP && ax < x + bw + GAP && y < ay + ah + GAP && ay < y + bh + GAP
            };
            if !overlap(bx, by) {
                return Ok(());
            }
            let monitor = unsafe { native::window_monitor(wins[0].handle, 0) };
            let room = f64::from(unsafe { native::monitor_width(monitor) });
            let to_x = if ax + aw + GAP + bw <= room {
                ax + aw + GAP
            } else {
                (ax - GAP - bw).max(0.0)
            };
            // Held where the first window does not cover the second.
            let grab = (bx + bw - 12.0, by + bh - 12.0);
            self.to(grab.0, grab.1)?;
            self.session
                .call::<_, _, ()>("NotifyPointerButton", &(BTN_LEFT, true))
                .map_err(fail("NotifyPointerButton"))?;
            let handle = wins[1].handle;
            let end = Instant::now() + Duration::from_secs(2);
            loop {
                let event = unsafe { native::window_wait(handle, 0.05) };
                if matches!(event, Event::MouseInput { .. }) {
                    break;
                }
                if Instant::now() >= end {
                    return Err("the button held on the second window never reached it".into());
                }
            }
            let dragged = unsafe { native::window_drag(handle) };
            let (dx, dy) = (to_x - bx, ay - by);
            for step in 1..=10 {
                let t = f64::from(step) / 10.0;
                self.to(grab.0 + dx * t, grab.1 + dy * t)?;
                for w in wins {
                    unsafe { native::window_wait(w.handle, 0.01) };
                }
            }
            self.session
                .call::<_, _, ()>("NotifyPointerButton", &(BTN_LEFT, false))
                .map_err(fail("NotifyPointerButton"))?;
            if !dragged {
                return Err("dragWindow refused to move the second window".into());
            }
            // The drag's own pointer events would misplace the window.
            std::thread::sleep(Duration::from_millis(300));
            for w in wins {
                while unsafe { native::window_poll(w.handle) } != Event::None {}
            }
            self.origins.clear();
            self.find(wins)?;
            let (bx, by, ..) = self.rect(&wins[1])?;
            if overlap(bx, by) {
                return Err(format!("dragWindow left the second window at {bx}, {by}"));
            }
            Ok(())
        }

        fn key_strokes(&self, keys: &[u32]) -> Result<(), String> {
            for &key in keys {
                self.session
                    .call::<_, _, ()>("NotifyKeyboardKeycode", &(key, true))
                    .map_err(fail("NotifyKeyboardKeycode"))?;
            }
            for &key in keys.iter().rev() {
                self.session
                    .call::<_, _, ()>("NotifyKeyboardKeycode", &(key, false))
                    .map_err(fail("NotifyKeyboardKeycode"))?;
            }
            Ok(())
        }
    }

    impl Input for Mutter {
        fn name(&self) -> &'static str {
            "mutter's RemoteDesktop under GNOME"
        }

        fn focus(&mut self, w: &Win) -> Option<Result<(), String>> {
            Some(self.point(w, 12.0, 12.0).and_then(|()| self.click()))
        }

        fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String> {
            let &(_, ox, oy) = self
                .origins
                .iter()
                .find(|(title, ..)| *title == w.title)
                .ok_or("an unfound window")?;
            self.to(ox + x, oy + y)
        }

        fn nudge(&mut self) -> Result<(), String> {
            self.session
                .call::<_, _, ()>("NotifyPointerMotionRelative", &(3.0f64, 2.0f64))
                .map_err(fail("NotifyPointerMotionRelative"))
        }

        fn click(&mut self) -> Result<(), String> {
            self.session
                .call::<_, _, ()>("NotifyPointerButton", &(BTN_LEFT, true))
                .map_err(fail("NotifyPointerButton"))?;
            self.session
                .call::<_, _, ()>("NotifyPointerButton", &(BTN_LEFT, false))
                .map_err(fail("NotifyPointerButton"))
        }

        /// A negative step scrolls up, as libinput counts.
        fn scroll_up(&mut self) -> Result<(), String> {
            self.session
                .call::<_, _, ()>("NotifyPointerAxisDiscrete", &(0u32, -1i32))
                .map_err(fail("NotifyPointerAxisDiscrete"))
        }

        fn key(&mut self, stroke: Stroke) -> Result<(), String> {
            match stroke {
                Stroke::A => self.key_strokes(&[KEY_A]),
                Stroke::ShiftB => self.key_strokes(&[KEY_LEFTSHIFT, KEY_B]),
                Stroke::Left => self.key_strokes(&[KEY_LEFT]),
            }
        }
    }
}

#[cfg(target_os = "linux")]
mod virtual_seat {
    use std::io::Write;
    use std::time::Instant;

    use wayland_client::globals::{GlobalListContents, registry_queue_init};
    use wayland_client::protocol::wl_pointer::{Axis, AxisSource, ButtonState};
    use wayland_client::protocol::{wl_registry, wl_seat::WlSeat};
    use wayland_client::{Connection, Dispatch, EventQueue, QueueHandle, delegate_noop};
    use wayland_protocols_misc::zwp_virtual_keyboard_v1::client::{
        zwp_virtual_keyboard_manager_v1::ZwpVirtualKeyboardManagerV1,
        zwp_virtual_keyboard_v1::ZwpVirtualKeyboardV1,
    };
    use wayland_protocols_wlr::virtual_pointer::v1::client::{
        zwlr_virtual_pointer_manager_v1::ZwlrVirtualPointerManagerV1,
        zwlr_virtual_pointer_v1::ZwlrVirtualPointerV1,
    };

    // evdev codes.
    pub const KEY_A: u32 = 30;
    pub const KEY_B: u32 = 48;
    pub const KEY_LEFTSHIFT: u32 = 42;
    pub const KEY_LEFT: u32 = 105;
    const BTN_LEFT: u32 = 0x110;
    /// The Shift modifier's mask in the keymap below.
    pub const SHIFT: u32 = 1;

    /// The keys the test types, at their evdev codes plus 8.
    const KEYMAP: &str = r#"xkb_keymap {
    xkb_keycodes "desktop" {
        minimum = 8;
        maximum = 255;
        <AC01> = 38;
        <AB05> = 56;
        <LFSH> = 50;
        <LEFT> = 113;
    };
    xkb_types "desktop" { include "complete" };
    xkb_compatibility "desktop" { include "complete" };
    xkb_symbols "desktop" {
        key <AC01> { [ a, A ] };
        key <AB05> { [ b, B ] };
        key <LFSH> { [ Shift_L ] };
        key <LEFT> { [ Left ] };
        modifier_map Shift { <LFSH> };
    };
};
"#;

    struct State;

    impl Dispatch<wl_registry::WlRegistry, GlobalListContents> for State {
        fn event(
            _: &mut Self,
            _: &wl_registry::WlRegistry,
            _: wl_registry::Event,
            _: &GlobalListContents,
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    delegate_noop!(State: ignore WlSeat);
    delegate_noop!(State: ZwlrVirtualPointerManagerV1);
    delegate_noop!(State: ZwlrVirtualPointerV1);
    delegate_noop!(State: ZwpVirtualKeyboardManagerV1);
    delegate_noop!(State: ZwpVirtualKeyboardV1);

    /// A virtual pointer and keyboard on a connection of the test's own.
    pub struct Seat {
        queue: EventQueue<State>,
        pointer: ZwlrVirtualPointerV1,
        keyboard: ZwpVirtualKeyboardV1,
        extent: (u32, u32),
        start: Instant,
    }

    impl Seat {
        /// `extent` is the output layout's size, which absolute motion spans.
        pub fn new(extent: (i32, i32)) -> Result<Self, String> {
            let fail = |e: &dyn std::fmt::Display| format!("virtual seat: {e}");
            let connection = Connection::connect_to_env().map_err(|e| fail(&e))?;
            let (globals, mut queue) =
                registry_queue_init::<State>(&connection).map_err(|e| fail(&e))?;
            let qh = queue.handle();
            let seat: WlSeat = globals.bind(&qh, 1..=7, ()).map_err(|e| fail(&e))?;
            let pointers: ZwlrVirtualPointerManagerV1 =
                globals.bind(&qh, 1..=2, ()).map_err(|e| fail(&e))?;
            let keyboards: ZwpVirtualKeyboardManagerV1 =
                globals.bind(&qh, 1..=1, ()).map_err(|e| fail(&e))?;
            let pointer = pointers.create_virtual_pointer(Some(&seat), &qh, ());
            let keyboard = keyboards.create_virtual_keyboard(&seat, &qh, ());

            let mut file = tempfile()?;
            file.write_all(KEYMAP.as_bytes()).map_err(|e| fail(&e))?;
            file.write_all(&[0]).map_err(|e| fail(&e))?;
            let size = KEYMAP.len() as u32 + 1;
            // 1 is XKB_V1.
            keyboard.keymap(1, std::os::fd::AsFd::as_fd(&file), size);
            let mut state = State;
            queue.roundtrip(&mut state).map_err(|e| fail(&e))?;
            Ok(Self {
                queue,
                pointer,
                keyboard,
                extent: (extent.0.max(1) as u32, extent.1.max(1) as u32),
                start: Instant::now(),
            })
        }

        fn time(&self) -> u32 {
            self.start.elapsed().as_millis() as u32
        }

        /// Sends what is queued and waits until the compositor has it.
        fn sync(&mut self) -> Result<(), String> {
            self.queue
                .roundtrip(&mut State)
                .map(|_| ())
                .map_err(|e| format!("virtual seat: {e}"))
        }

        pub fn point(&mut self, x: f64, y: f64) -> Result<(), String> {
            let (w, h) = self.extent;
            let (x, y) = (x.clamp(0.0, w as f64) as u32, y.clamp(0.0, h as f64) as u32);
            self.pointer.motion_absolute(self.time(), x, y, w, h);
            self.pointer.frame();
            self.sync()
        }

        pub fn nudge(&mut self, dx: f64, dy: f64) -> Result<(), String> {
            self.pointer.motion(self.time(), dx, dy);
            self.pointer.frame();
            self.sync()
        }

        pub fn click(&mut self) -> Result<(), String> {
            for state in [ButtonState::Pressed, ButtonState::Released] {
                self.pointer.button(self.time(), BTN_LEFT, state);
                self.pointer.frame();
            }
            self.sync()
        }

        pub fn scroll_up(&mut self) -> Result<(), String> {
            let time = self.time();
            self.pointer.axis_source(AxisSource::Wheel);
            self.pointer
                .axis_discrete(time, Axis::VerticalScroll, -15.0, -1);
            self.pointer.frame();
            self.sync()
        }

        /// Presses each key in turn with the modifiers it is pressed under,
        /// then releases them in reverse.
        pub fn keys(&mut self, keys: &[(u32, u32)]) -> Result<(), String> {
            for &(key, mods) in keys {
                self.keyboard.key(self.time(), key, 1);
                self.keyboard.modifiers(mods, 0, 0, 0);
            }
            for (i, &(key, _)) in keys.iter().enumerate().rev() {
                self.keyboard.key(self.time(), key, 0);
                let mods = if i == 0 { 0 } else { keys[i - 1].1 };
                self.keyboard.modifiers(mods, 0, 0, 0);
            }
            self.sync()
        }
    }

    fn tempfile() -> Result<std::fs::File, String> {
        let path = std::env::temp_dir().join(format!("xwindow-desktop-{}.xkb", std::process::id()));
        let file = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(&path)
            .map_err(|e| format!("virtual seat keymap: {e}"))?;
        let _ = std::fs::remove_file(&path);
        Ok(file)
    }
}

#[cfg(windows)]
mod windows_input {
    use windows_sys::Win32::Foundation::POINT;
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
        INPUT, INPUT_0, INPUT_KEYBOARD, INPUT_MOUSE, KEYBDINPUT, KEYEVENTF_EXTENDEDKEY,
        KEYEVENTF_KEYUP, KEYEVENTF_SCANCODE, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP,
        MOUSEEVENTF_MOVE, MOUSEEVENTF_WHEEL, MOUSEINPUT, SendInput as send,
    };
    use windows_sys::Win32::UI::WindowsAndMessaging::SetCursorPos;

    use super::{Input, Stroke, Win, native};

    /// Injected input, which Windows routes as a real device's.
    pub struct SendInput;

    fn inject(inputs: &[INPUT]) -> Result<(), String> {
        let sent = unsafe {
            send(
                inputs.len() as u32,
                inputs.as_ptr(),
                size_of::<INPUT>() as i32,
            )
        };
        if sent as usize == inputs.len() {
            Ok(())
        } else {
            Err(format!(
                "SendInput sent {sent} of {}: {}",
                inputs.len(),
                std::io::Error::last_os_error()
            ))
        }
    }

    fn mouse(flags: u32, dx: i32, dy: i32, data: u32) -> INPUT {
        INPUT {
            r#type: INPUT_MOUSE,
            Anonymous: INPUT_0 {
                mi: MOUSEINPUT {
                    dx,
                    dy,
                    mouseData: data,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    fn key(scan: u16, extended: bool, up: bool) -> INPUT {
        let mut flags = KEYEVENTF_SCANCODE;
        if extended {
            flags |= KEYEVENTF_EXTENDEDKEY;
        }
        if up {
            flags |= KEYEVENTF_KEYUP;
        }
        INPUT {
            r#type: INPUT_KEYBOARD,
            Anonymous: INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: 0,
                    wScan: scan,
                    dwFlags: flags,
                    time: 0,
                    dwExtraInfo: 0,
                },
            },
        }
    }

    // Set 1 scan codes.
    const SCAN_A: u16 = 0x1e;
    const SCAN_B: u16 = 0x30;
    const SCAN_LSHIFT: u16 = 0x2a;
    const SCAN_LEFT: u16 = 0x4b;
    const WHEEL_DELTA: u32 = 120;

    impl Input for SendInput {
        fn name(&self) -> &'static str {
            "SendInput"
        }

        fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String> {
            let hwnd = unsafe { native::window_raw(w.handle, 0) } as isize;
            let scale = unsafe { native::window_scale_factor(w.handle) };
            let mut at = POINT {
                x: (x * scale).round() as i32,
                y: (y * scale).round() as i32,
            };
            if unsafe { ClientToScreen(hwnd as _, &mut at) } == 0 {
                return Err("ClientToScreen failed".into());
            }
            if unsafe { SetCursorPos(at.x, at.y) } == 0 {
                return Err(format!("SetCursorPos: {}", std::io::Error::last_os_error()));
            }
            // A zero move after the jump, so the move is seen as input too.
            inject(&[mouse(MOUSEEVENTF_MOVE, 0, 0, 0)])
        }

        fn nudge(&mut self) -> Result<(), String> {
            inject(&[mouse(MOUSEEVENTF_MOVE, 3, 2, 0)])
        }

        fn click(&mut self) -> Result<(), String> {
            inject(&[
                mouse(MOUSEEVENTF_LEFTDOWN, 0, 0, 0),
                mouse(MOUSEEVENTF_LEFTUP, 0, 0, 0),
            ])
        }

        fn scroll_up(&mut self) -> Result<(), String> {
            inject(&[mouse(MOUSEEVENTF_WHEEL, 0, 0, WHEEL_DELTA)])
        }

        fn key(&mut self, stroke: Stroke) -> Result<(), String> {
            match stroke {
                Stroke::A => inject(&[key(SCAN_A, false, false), key(SCAN_A, false, true)]),
                Stroke::ShiftB => inject(&[
                    key(SCAN_LSHIFT, false, false),
                    key(SCAN_B, false, false),
                    key(SCAN_B, false, true),
                    key(SCAN_LSHIFT, false, true),
                ]),
                Stroke::Left => inject(&[key(SCAN_LEFT, true, false), key(SCAN_LEFT, true, true)]),
            }
        }
    }
}

/// macOS through Quartz events posted at the HID level, which need the
/// Accessibility permission for the terminal that runs the test.
#[cfg(target_os = "macos")]
mod quartz {
    use std::ffi::c_void;

    use super::{Input, Stroke, Win, native};

    #[repr(C)]
    #[derive(Clone, Copy)]
    struct CGPoint {
        x: f64,
        y: f64,
    }

    type Event = *mut c_void;

    #[link(name = "CoreGraphics", kind = "framework")]
    unsafe extern "C" {
        fn CGEventCreate(source: *const c_void) -> Event;
        fn CGEventGetLocation(event: Event) -> CGPoint;
        fn CGEventCreateMouseEvent(
            source: *const c_void,
            kind: u32,
            at: CGPoint,
            button: u32,
        ) -> Event;
        fn CGEventCreateKeyboardEvent(source: *const c_void, key: u16, down: bool) -> Event;
        fn CGEventCreateScrollWheelEvent2(
            source: *const c_void,
            units: u32,
            count: u32,
            wheel1: i32,
            wheel2: i32,
            wheel3: i32,
        ) -> Event;
        fn CGEventSetFlags(event: Event, flags: u64);
        fn CGEventSetIntegerValueField(event: Event, field: u32, value: i64);
        fn CGEventPost(tap: u32, event: Event);
    }

    #[link(name = "CoreFoundation", kind = "framework")]
    unsafe extern "C" {
        fn CFRelease(object: *const c_void);
    }

    const HID: u32 = 0;
    const LEFT_MOUSE_DOWN: u32 = 1;
    const LEFT_MOUSE_UP: u32 = 2;
    const MOUSE_MOVED: u32 = 5;
    const LEFT: u32 = 0;
    const PIXEL: u32 = 0;
    const DELTA_X: u32 = 4;
    const DELTA_Y: u32 = 5;
    const SHIFT: u64 = 0x20000;
    // Virtual key codes.
    const KEY_A: u16 = 0x00;
    const KEY_B: u16 = 0x0b;
    const KEY_SHIFT: u16 = 0x38;
    const KEY_LEFT: u16 = 0x7b;

    /// Posts `event` and releases it.
    fn post(event: Event) -> Result<(), String> {
        if event.is_null() {
            return Err("Quartz made no event".into());
        }
        unsafe {
            CGEventPost(HID, event);
            CFRelease(event);
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
        Ok(())
    }

    fn pointer() -> CGPoint {
        unsafe {
            let event = CGEventCreate(std::ptr::null());
            let at = CGEventGetLocation(event);
            CFRelease(event);
            at
        }
    }

    fn mouse(kind: u32, at: CGPoint) -> Event {
        unsafe { CGEventCreateMouseEvent(std::ptr::null(), kind, at, LEFT) }
    }

    fn key(code: u16, down: bool, flags: u64) -> Result<(), String> {
        let event = unsafe { CGEventCreateKeyboardEvent(std::ptr::null(), code, down) };
        if !event.is_null() {
            unsafe { CGEventSetFlags(event, flags) };
        }
        post(event)
    }

    pub struct Quartz;

    impl Input for Quartz {
        fn name(&self) -> &'static str {
            "Quartz"
        }

        fn point(&mut self, w: &Win, x: f64, y: f64) -> Result<(), String> {
            // Global points, from the top left of the main display.
            let scale = unsafe { native::window_scale_factor(w.handle) };
            let (left, top) = unsafe {
                (
                    native::window_inner_x(w.handle) as f64 / scale,
                    native::window_inner_y(w.handle) as f64 / scale,
                )
            };
            post(mouse(
                MOUSE_MOVED,
                CGPoint {
                    x: left + x,
                    y: top + y,
                },
            ))
        }

        fn nudge(&mut self) -> Result<(), String> {
            let at = pointer();
            let event = mouse(
                MOUSE_MOVED,
                CGPoint {
                    x: at.x + 3.0,
                    y: at.y + 2.0,
                },
            );
            if !event.is_null() {
                unsafe {
                    CGEventSetIntegerValueField(event, DELTA_X, 3);
                    CGEventSetIntegerValueField(event, DELTA_Y, 2);
                }
            }
            post(event)
        }

        fn click(&mut self) -> Result<(), String> {
            let at = pointer();
            post(mouse(LEFT_MOUSE_DOWN, at))?;
            post(mouse(LEFT_MOUSE_UP, at))
        }

        fn scroll_up(&mut self) -> Result<(), String> {
            post(unsafe { CGEventCreateScrollWheelEvent2(std::ptr::null(), PIXEL, 1, 10, 0, 0) })
        }

        fn key(&mut self, stroke: Stroke) -> Result<(), String> {
            match stroke {
                Stroke::A => {
                    key(KEY_A, true, 0)?;
                    key(KEY_A, false, 0)
                }
                Stroke::ShiftB => {
                    key(KEY_SHIFT, true, SHIFT)?;
                    key(KEY_B, true, SHIFT)?;
                    key(KEY_B, false, SHIFT)?;
                    key(KEY_SHIFT, false, 0)
                }
                Stroke::Left => {
                    key(KEY_LEFT, true, 0)?;
                    key(KEY_LEFT, false, 0)
                }
            }
        }
    }
}
