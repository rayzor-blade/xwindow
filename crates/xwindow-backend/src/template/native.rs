//! Windows natively: winit.
//!
//! Every window shares one event loop on the thread that opened the first;
//! winit allows one per program, and some platforms require it to be the
//! main thread. Its events are sorted into each window's queue, so polling
//! one window never loses another's events. Raw device events go to the
//! focused window, or the first open one.
//!
//! The loop is driven one of two ways, which the host picks with `attach`:
//!
//! - The program pumps it inside `open`, `poll` and `wait`. This is the
//!   default, on every platform winit pumps on.
//! - The host runs it and gives the program turns. iOS has no pumping, so
//!   it is the only way there.
//!
//! Uses its adapter's generated model at the crate root and its carriers
//! from `crate::runtime`.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, VecDeque};
use std::ptr::NonNull;
use std::sync::OnceLock;
use std::sync::atomic::{AtomicBool, Ordering};

static WAKE_PROXY: OnceLock<winit::event_loop::EventLoopProxy<()>> = OnceLock::new();
static WORK_PENDING: AtomicBool = AtomicBool::new(false);
use std::time::{Duration, Instant};

use raw_window_handle::{HasDisplayHandle, HasWindowHandle};
use winit::application::ApplicationHandler;
use winit::dpi::{LogicalPosition, LogicalSize};
use winit::event::{self as native, DeviceId, WindowEvent};
use winit::event_loop::{
    ActiveEventLoop, AsyncRequestSerial, ControlFlow, DeviceEvents as Listen, EventLoop,
};
use winit::keyboard as keys;
use winit::monitor::{MonitorHandle, VideoModeHandle};
#[cfg(not(target_os = "ios"))]
use winit::platform::pump_events::EventLoopExtPumpEvents;
use winit::window::{self as windows, WindowId};
use xwindow_core::{Kind, Slab};

use crate::runtime::{Buffer, ErrorKind, Text, host};
use crate::{
    Attention, CursorGrab, CursorIcon, CursorRange, DeviceEvent, DeviceEvents, Event, FilePath,
    Ime, ImePurpose, Key, KeyCode, KeyEvent, KeyLocation, KeySupplement, Modifiers,
    ModifiersKeyState, MouseButton, MouseElementState, MouseScrollDelta, NamedKey, NativeKey,
    NativeKeyCode, OptionalBytes, OptionalFloat, OptionalText, PhysicalKey, ResizeDirection,
    ScaleSizing, Theme, TouchForce, TouchPhase, VariantBytes, WindowAttributes, WindowLevel,
};

struct Open {
    window: windows::Window,
    events: VecDeque<Event>,
    sizing: ScaleSizing,
    /// What the program set: winit reads no title back on X11 and others.
    title: String,
    /// The platform was asked for events since this window last answered a
    /// poll or wait with none.
    pumped: bool,
    /// A redraw the program asked for, on platforms where xwindow delivers
    /// it rather than winit.
    redraw: Redraw,
}

/// A redraw comes, as winit's does, once the platform has had a pass since
/// the program asked for it, after the events that pass brought.
#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum Redraw {
    #[default]
    None,
    /// Asked for since the platform's last pass.
    Asked,
    /// Asked for before it: due after the window's queued events.
    Due,
}

struct App {
    windows: Slab<Open>,
    ids: HashMap<WindowId, i32>,
    /// Windows asked for and not yet made, with how each sizes on a change
    /// of scale. winit makes them only while it runs.
    opening: Vec<(windows::WindowAttributes, ScaleSizing)>,
    /// What became of each, in order: its handle, or why there is none.
    opened: Vec<Result<i32, String>>,
    resumed: bool,
    focused: Option<i32>,
    devices: HashMap<DeviceId, i32>,
    requests: Vec<(AsyncRequestSerial, i64)>,
    next_request: i64,
    monitors: Vec<MonitorHandle>,
    video_modes: Vec<VideoModeHandle>,
    /// Under a host, when the program asked for its next turn.
    next_turn: ControlFlow,
    /// When raw device events come; none are kept under `Never`.
    listen: Listen,
    /// Connected through an open window's display, which the event loop
    /// owns, so it outlives the clipboard.
    clipboard: Option<xwindow_clipboard::Clipboard>,
    /// The types `clipboardTypeCount` found, which `clipboardType` names.
    clipboard_types: Vec<String>,
}

/// Who drives the loop.
enum Driver {
    /// The program, by pumping it.
    #[cfg(not(target_os = "ios"))]
    Pumped(EventLoop<()>),
    /// The host, which runs it and gives the program turns.
    Hosted,
}

// Fields drop in order: the windows and the clipboard, which use the event
// loop's display connection, go before the loop.
struct Loop {
    app: App,
    driver: Driver,
}

thread_local! {
    static LOOP: RefCell<Option<Loop>> = const { RefCell::new(None) };
    /// The running loop, while the host gives the program a turn.
    static ACTIVE: Cell<Option<NonNull<ActiveEventLoop>>> = const { Cell::new(None) };
    /// When device events should come, until the loop next runs and takes it.
    static LISTEN: Cell<Option<Listen>> = const { Cell::new(None) };
    static EXTERNAL_PUMP: Cell<bool> = const { Cell::new(false) };
}

impl App {
    fn new() -> Self {
        App {
            windows: Slab::new(Kind::Window),
            ids: HashMap::new(),
            opening: Vec::new(),
            opened: Vec::new(),
            resumed: false,
            focused: None,
            devices: HashMap::new(),
            requests: Vec::new(),
            next_request: 1,
            monitors: Vec::new(),
            video_modes: Vec::new(),
            next_turn: ControlFlow::Wait,
            listen: Listen::default(),
            clipboard: None,
            clipboard_types: Vec::new(),
        }
    }
}

/// Runs `body` on the event loop, made on first use, or returns `miss`.
fn with<T>(miss: T, body: impl FnOnce(&mut Loop) -> T) -> T {
    LOOP.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            host::raise(
                ErrorKind::Runtime,
                "window operations cannot re-enter the event loop",
            );
            return miss;
        };
        if slot.is_none() {
            // On Android, with the AndroidApp an adapter's android_main kept.
            #[cfg(not(target_os = "ios"))]
            match c_event_loop() {
                Ok(events) => {
                    let _ = WAKE_PROXY.set(events.create_proxy());
                    *slot = Some(Loop {
                        driver: Driver::Pumped(events),
                        app: App::new(),
                    })
                }
                Err(error) => {
                    host::raise(ErrorKind::Runtime, &error);
                    return miss;
                }
            }
            #[cfg(target_os = "ios")]
            {
                host::raise(
                    ErrorKind::Runtime,
                    "window: on iOS the host runs the event loop, and opens windows in the program's turns",
                );
                return miss;
            }
        }
        body(slot.as_mut().expect("made above"))
    })
}

/// The running loop, during a turn the host gives.
fn active<T>(body: impl FnOnce(&ActiveEventLoop) -> T) -> Option<T> {
    // Set only for the length of the callback that lent it.
    ACTIVE.get().map(|active| body(unsafe { active.as_ref() }))
}

// -- the host's hook ----------------------------------------------------------

/// How a host has the program driven, given to `attach`.
pub enum Drive<'a> {
    /// The program pumps the loop inside `open`, `poll` and `wait`, as it
    /// does when no host attaches one. winit cannot pump on iOS.
    #[cfg(not(target_os = "ios"))]
    Pump,
    /// The host runs the loop, and calls `turn` each time it has sorted the
    /// events that came: the program's turn, until `turn` returns false.
    ///
    /// A window opens only in a turn. `poll` and `wait` return what has
    /// come without waiting, and the last that finds nothing says when the
    /// next turn is: `poll` at once, `wait` within its timeout, and a
    /// negative `wait` when events come.
    Turns(&'a mut dyn FnMut() -> bool),
}

/// The host's hook: hands the backend the event loop every window shares.
/// Call it on the thread that will open windows, before the first opens.
/// On Android the host builds `events` with its `AndroidApp`.
///
/// `Drive::Pump` returns at once. `Drive::Turns` runs the loop and returns
/// when it ends, which on iOS it never does.
pub fn attach(events: EventLoop<()>, drive: Drive<'_>) -> Result<(), String> {
    let proxy = events.create_proxy();
    let (driver, hosted) = match drive {
        #[cfg(not(target_os = "ios"))]
        Drive::Pump => (Driver::Pumped(events), None),
        Drive::Turns(turn) => (Driver::Hosted, Some((events, turn))),
    };
    LOOP.with(|cell| {
        let Ok(mut slot) = cell.try_borrow_mut() else {
            return Err("window: attach cannot re-enter the event loop".to_owned());
        };
        if slot.is_some() {
            return Err("window: the event loop is already made".to_owned());
        }
        *slot = Some(Loop {
            driver,
            app: App::new(),
        });
        Ok(())
    })?;
    let _ = WAKE_PROXY.set(proxy);
    match hosted {
        Some((events, turn)) => events
            .run_app(&mut Host {
                turn,
                came: false,
                done: false,
            })
            .map_err(|error| format!("window: {error}")),
        None => Ok(()),
    }
}

/// A runtime with its own I/O loop can wait in `pump_external` instead of
/// periodically polling each window. While enabled, nonblocking window polls
/// only drain the queues; the runtime is responsible for pumping on this thread.
pub fn external_pump(enabled: bool) -> Result<(), String> {
    if enabled {
        if EXTERNAL_PUMP.get() {
            return Err("window: an external event pump is already installed".to_owned());
        }
        if !with(false, |l| !l.hosted()) {
            return Err("window: external pumping requires a pumped event loop".to_owned());
        }
    }
    EXTERNAL_PUMP.set(enabled);
    Ok(())
}

/// A thread-safe wake signal for an external runtime's I/O readiness watcher.
#[cfg(not(target_os = "ios"))]
pub fn external_waker() -> Option<winit::event_loop::EventLoopProxy<()>> {
    with(None, |l| match &l.driver {
        Driver::Pumped(events) => Some(events.create_proxy()),
        Driver::Hosted => None,
    })
}

fn pending(app: &App) -> bool {
    app.windows
        .iter()
        .any(|(_, open)| !open.events.is_empty() || open.redraw == Redraw::Due)
}

pub fn external_pending() -> bool {
    WORK_PENDING.load(Ordering::Acquire) || with(false, |l| pending(&l.app))
}

/// Wait for platform events once, or return immediately for events already
/// queued. A proxy wake also returns, even if there is no window event to deliver.
pub fn pump_external(timeout: Option<Duration>) -> bool {
    with(false, |l| {
        let woke = WORK_PENDING.swap(false, Ordering::AcqRel);
        if !woke && !pending(&l.app) {
            let asked = l.app.windows.iter().any(|(_, open)| open.redraw == Redraw::Asked);
            l.pump(if asked { Some(Duration::ZERO) } else { timeout });
        }
        let came = WORK_PENDING.swap(false, Ordering::AcqRel);
        woke || came || pending(&l.app)
    })
}

// -- the hook for a host in C ---------------------------------------------------
//
// A host that links its adapter from C, such as an app that links HashLink
// statically on a phone, reaches `attach` through these. They report failure
// by status and on stderr: the program's runtime may not be running yet.

/// The `AndroidApp` the adapter's `android_main` was given, for the loop the
/// C hook builds.
#[cfg(target_os = "android")]
static ANDROID_APP: std::sync::Mutex<Option<winit::platform::android::activity::AndroidApp>> =
    std::sync::Mutex::new(None);

/// Keeps `app` for the C hook. An adapter that defines `android_main` for a
/// host in C calls this before it starts the host's program.
#[cfg(target_os = "android")]
pub fn android_app(app: winit::platform::android::activity::AndroidApp) {
    *ANDROID_APP.lock().unwrap_or_else(|e| e.into_inner()) = Some(app);
}

/// An event loop for the backend to make itself, or for the C hook to hand
/// over: on Android, built with the kept `AndroidApp`.
fn c_event_loop() -> Result<EventLoop<()>, String> {
    #[cfg(target_os = "android")]
    {
        use winit::platform::android::EventLoopBuilderExtAndroid;
        let app = ANDROID_APP
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .take()
            .ok_or("window: no AndroidApp; the adapter's android_main keeps it")?;
        EventLoop::builder()
            .with_android_app(app)
            .build()
            .map_err(|error| format!("window: {error}"))
    }
    #[cfg(not(target_os = "android"))]
    EventLoop::new().map_err(|error| format!("window: {error}"))
}

fn c_status(result: Result<(), String>) -> i32 {
    match result {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            -1
        }
    }
}

/// `attach` with `Drive::Pump`: the program pumps the loop. 0 when attached,
/// -1 when not; never on iOS, which cannot pump.
#[unsafe(no_mangle)]
pub extern "C" fn xwindow_attach_pump() -> i32 {
    #[cfg(not(target_os = "ios"))]
    let result = c_event_loop().and_then(|events| attach(events, Drive::Pump));
    #[cfg(target_os = "ios")]
    let result = Err("window: iOS cannot pump; run the loop with xwindow_run_turns".to_owned());
    c_status(result)
}

/// `attach` with `Drive::Turns`: runs the loop, calling `turn(data)` for each
/// of the program's turns until it returns 0. Returns 0 when the loop ends,
/// which on iOS it never does, and -1 when it could not run.
///
/// # Safety
///
/// `turn` is called with `data` on this thread for as long as the loop runs.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn xwindow_run_turns(
    turn: extern "C" fn(*mut std::ffi::c_void) -> i32,
    data: *mut std::ffi::c_void,
) -> i32 {
    let mut turn = || turn(data) != 0;
    c_status(c_event_loop().and_then(|events| attach(events, Drive::Turns(&mut turn))))
}

/// The handler a host's loop runs: the backend's, and the program's turns.
struct Host<'a> {
    turn: &'a mut dyn FnMut() -> bool,
    /// Something has come since the program's last turn. winit wakes for
    /// more than the program's events, and a wake with none is no turn.
    came: bool,
    /// The program has ended. iOS cannot end its loop, so the loop waits.
    done: bool,
}

impl Host<'_> {
    /// Runs `body` on the backend's handler. winit does not re-enter its
    /// handler, and the program's turn is not in one, so it is free.
    fn app(&mut self, body: impl FnOnce(&mut App)) {
        LOOP.with(|cell| {
            if let Ok(mut slot) = cell.try_borrow_mut() {
                if let Some(l) = slot.as_mut() {
                    body(&mut l.app);
                }
            }
        });
    }
}

impl ApplicationHandler for Host<'_> {
    fn user_event(&mut self, event_loop: &ActiveEventLoop, event: ()) {
        self.came = true;
        self.app(|app| app.user_event(event_loop, event));
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, cause: native::StartCause) {
        self.app(|app| {
            if let Some(listen) = LISTEN.take() {
                event_loop.listen_device_events(listen);
                app.listen = listen;
            }
            app.new_events(event_loop, cause)
        });
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.came = true;
        self.app(|app| app.resumed(event_loop));
    }

    fn suspended(&mut self, event_loop: &ActiveEventLoop) {
        self.came = true;
        self.app(|app| app.suspended(event_loop));
    }

    fn memory_warning(&mut self, event_loop: &ActiveEventLoop) {
        self.came = true;
        self.app(|app| app.memory_warning(event_loop));
    }

    fn window_event(&mut self, event_loop: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        self.came = true;
        self.app(|app| app.window_event(event_loop, id, event));
    }

    fn device_event(
        &mut self,
        event_loop: &ActiveEventLoop,
        id: DeviceId,
        event: native::DeviceEvent,
    ) {
        self.came = true;
        self.app(|app| app.device_event(event_loop, id, event));
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        let (mut resumed, mut asked) = (false, ControlFlow::Wait);
        self.app(|app| (resumed, asked) = (app.resumed, app.next_turn));
        // A window has nowhere to open before the platform resumes.
        if !resumed || self.done {
            return;
        }
        let due = match asked {
            ControlFlow::Poll => true,
            ControlFlow::Wait => false,
            ControlFlow::WaitUntil(deadline) => Instant::now() >= deadline,
        };
        if !self.came && !due {
            event_loop.set_control_flow(asked);
            return;
        }
        self.came = false;
        self.app(|app| {
            app.next_turn = ControlFlow::Wait;
            app.passed();
        });
        ACTIVE.set(Some(NonNull::from(event_loop)));
        self.done = !(self.turn)();
        ACTIVE.set(None);
        if self.done {
            event_loop.set_control_flow(ControlFlow::Wait);
            event_loop.exit();
            return;
        }
        let mut next = ControlFlow::Wait;
        self.app(|app| next = app.next_turn);
        event_loop.set_control_flow(next);
    }
}

/// Runs `body` on the open window `handle`, or returns `miss`.
fn window<T>(handle: i32, miss: T, body: impl FnOnce(&windows::Window) -> T) -> T {
    let mut body = Some(body);
    let mut miss = Some(miss);
    let found = with(None, |l| {
        l.app
            .windows
            .get(handle)
            .map(|open| (body.take().expect("called once"))(&open.window))
    });
    found.unwrap_or_else(|| miss.take().expect("taken once"))
}

impl Loop {
    fn pump(&mut self, timeout: Option<Duration>) {
        match &mut self.driver {
            #[cfg(not(target_os = "ios"))]
            Driver::Pumped(events) => {
                if let Some(listen) = LISTEN.take() {
                    events.listen_device_events(listen);
                    self.app.listen = listen;
                }
                // A run of the application on macOS returns only once AppKit
                // dequeues an event after it is stopped, and AppKit can miss
                // one posted while it waits; xwindow serves the run itself.
                #[cfg(target_os = "macos")]
                let timeout = if appkit::serve(timeout) {
                    None
                } else {
                    timeout
                };
                // The pump only queues events in Rust's memory and calls
                // nothing in the program, so the runtime may collect while
                // it waits.
                {
                    let _blocking = Blocking::new();
                    events.pump_app_events(timeout, &mut self.app);
                }
                for (_, open) in self.app.windows.iter_mut() {
                    open.pumped = true;
                }
                self.app.passed();
            }
            Driver::Hosted => {
                let _ = timeout;
            }
        }
    }

    fn hosted(&self) -> bool {
        matches!(self.driver, Driver::Hosted)
    }

    /// The next event `handle` has, pumping once when it has none, and
    /// waiting until `deadline` for one when there is a deadline. Under a
    /// host, it asks for the next turn instead.
    ///
    /// A poll pumps only if this window has answered none since the last
    /// pump: a pump can take a display frame, and a program draining its
    /// events with polls would otherwise pump for as long as input lasts.
    fn next(&mut self, handle: i32, deadline: Option<Option<Instant>>) -> Event {
        let Some(open) = self.app.windows.get_mut(handle) else {
            return Event::None;
        };
        if deadline.is_none()
            && open.events.is_empty()
            && open.redraw == Redraw::None
            && open.pumped
        {
            open.pumped = false;
            return Event::None;
        }
        loop {
            // Polling must not consume a wake intended for the next blocking wait.
            if deadline.is_some() && WORK_PENDING.swap(false, Ordering::AcqRel) {
                return Event::None;
            }
            if let Some(open) = self.app.windows.get_mut(handle) {
                if let Some(event) = open.events.pop_front() {
                    return event;
                }
                if open.redraw == Redraw::Due {
                    open.redraw = Redraw::None;
                    return Event::RedrawRequested;
                }
            }
            if deadline.is_none() && EXTERNAL_PUMP.get() {
                return Event::None;
            }
            // A redraw asked for wants the platform's next pass now.
            let asked = self
                .app
                .windows
                .get(handle)
                .is_some_and(|open| open.redraw == Redraw::Asked);
            if self.hosted() {
                self.app.next_turn = match deadline {
                    _ if asked => ControlFlow::Poll,
                    None => ControlFlow::Poll,
                    Some(None) => ControlFlow::Wait,
                    Some(Some(deadline)) => ControlFlow::WaitUntil(deadline),
                };
                return Event::None;
            }
            let timeout = match deadline {
                _ if asked => Some(Duration::ZERO),
                None => Some(Duration::ZERO),
                Some(None) => None,
                Some(Some(deadline)) => Some(deadline.saturating_duration_since(Instant::now())),
            };
            self.pump(timeout);
            let waited_out = asked
                || match deadline {
                    None => true,
                    Some(None) => false,
                    Some(Some(deadline)) => Instant::now() >= deadline,
                };
            if waited_out || self.app.windows.get(handle).is_none() {
                let Some(open) = self.app.windows.get_mut(handle) else {
                    return Event::None;
                };
                if let Some(event) = open.events.pop_front() {
                    return event;
                }
                if open.redraw == Redraw::Due {
                    open.redraw = Redraw::None;
                    return Event::RedrawRequested;
                }
                open.pumped = false;
                return Event::None;
            }
        }
    }
}

/// What a call on a window can change that the program hears of.
#[cfg(target_os = "macos")]
#[derive(Clone, Copy, PartialEq)]
struct State {
    size: winit::dpi::PhysicalSize<u32>,
    position: Option<winit::dpi::PhysicalPosition<i32>>,
    focused: bool,
}

#[cfg(target_os = "macos")]
impl State {
    fn of(window: &windows::Window) -> Self {
        State {
            size: window.inner_size(),
            position: window.outer_position().ok(),
            focused: window.has_focus(),
        }
    }
}

impl App {
    /// Hands window `handle` a winit event, as winit does.
    fn deliver(&mut self, handle: i32, mut event: WindowEvent) {
        if let WindowEvent::ScaleFactorChanged {
            inner_size_writer, ..
        } = &mut event
        {
            if let Some(open) = self.windows.get(handle) {
                if open.sizing == ScaleSizing::Physical {
                    let _ = inner_size_writer.request_inner_size(open.window.inner_size());
                }
            }
        }
        if let WindowEvent::Focused(focused) = event {
            if focused {
                self.focused = Some(handle);
            } else if self.focused == Some(handle) {
                self.focused = None;
            }
        }
        let event = window_event(self, event);
        if let Some(open) = self.windows.get_mut(handle) {
            open.events.push_back(event);
        }
    }

    /// Each window's size, position and focus, to compare after a call.
    #[cfg(target_os = "macos")]
    fn states(&self) -> Vec<(i32, State)> {
        self.windows
            .iter()
            .map(|(handle, open)| (handle, State::of(&open.window)))
            .collect()
    }

    /// Delivers what changed since `before`, as the platform's events
    /// would have.
    #[cfg(target_os = "macos")]
    fn changed(&mut self, before: Vec<(i32, State)>) {
        for (handle, was) in before {
            let Some(open) = self.windows.get(handle) else {
                continue;
            };
            let now = State::of(&open.window);
            if now.size != was.size {
                self.deliver(handle, WindowEvent::Resized(now.size));
            }
            if now.position != was.position
                && let Some(position) = now.position
            {
                self.deliver(handle, WindowEvent::Moved(position));
            }
            if now.focused != was.focused {
                self.deliver(handle, WindowEvent::Focused(now.focused));
            }
        }
    }

    /// The platform had a pass: redraws asked for before it are due.
    fn passed(&mut self) {
        for (_, open) in self.windows.iter_mut() {
            if open.redraw == Redraw::Asked {
                open.redraw = Redraw::Due;
            }
        }
    }

    fn open_waiting(&mut self, event_loop: &ActiveEventLoop) {
        for (attributes, sizing) in std::mem::take(&mut self.opening) {
            let title = attributes.title.clone();
            let made = match event_loop.create_window(attributes) {
                Ok(window) => {
                    let id = window.id();
                    let handle = self.windows.put(Open {
                        window,
                        events: VecDeque::new(),
                        sizing,
                        title,
                        pumped: false,
                        redraw: Redraw::None,
                    });
                    self.ids.insert(id, handle);
                    Ok(handle)
                }
                Err(error) => Err(error.to_string()),
            };
            self.opened.push(made);
        }
        // Wayland sets a selection only with a recent input serial, which
        // the clipboard sees only from when it connects.
        self.clipboard();
    }

    /// The clipboard, connected through the first window to open.
    fn clipboard(&mut self) -> Option<&mut xwindow_clipboard::Clipboard> {
        if self.clipboard.is_none() {
            let (_, open) = self.windows.iter().next()?;
            let display = open.window.display_handle().ok()?.as_raw();
            self.clipboard = unsafe { xwindow_clipboard::Clipboard::connect(display) }.ok();
        }
        self.clipboard.as_mut()
    }

    fn device(&mut self, id: DeviceId) -> i32 {
        let next = self.devices.len() as i32 + 1;
        *self.devices.entry(id).or_insert(next)
    }

    /// The window raw device events go to.
    fn device_target(&self) -> Option<i32> {
        self.focused
            .filter(|handle| self.windows.get(*handle).is_some())
            .or_else(|| self.windows.iter().next().map(|(handle, _)| handle))
    }

    fn everywhere(&mut self, event: Event) {
        for (_, open) in self.windows.iter_mut() {
            open.events.push_back(event.clone());
        }
    }

    fn request(&mut self, serial: AsyncRequestSerial) -> i64 {
        if let Some((_, id)) = self.requests.iter().find(|(s, _)| *s == serial) {
            return *id;
        }
        let id = self.next_request;
        self.next_request += 1;
        self.requests.push((serial, id));
        id
    }

    fn monitor(&mut self, monitor: Option<MonitorHandle>) -> i32 {
        let Some(monitor) = monitor else { return 0 };
        let index = match self.monitors.iter().position(|m| *m == monitor) {
            Some(index) => index,
            None => {
                self.monitors.push(monitor);
                self.monitors.len() - 1
            }
        };
        monitor_handle(index)
    }

    fn video_mode(&mut self, mode: Option<VideoModeHandle>) -> i32 {
        let Some(mode) = mode else { return 0 };
        let index = match self.video_modes.iter().position(|m| *m == mode) {
            Some(index) => index,
            None => {
                self.video_modes.push(mode);
                self.video_modes.len() - 1
            }
        };
        kept_handle(Kind::VideoMode, index)
    }
}

impl ApplicationHandler for App {
    fn user_event(&mut self, _: &ActiveEventLoop, _: ()) {
        // The AppKit monitor may be waiting inside nextEvent, so hand it an
        // actual queue event as well as waking CoreFoundation through the proxy.
        #[cfg(target_os = "macos")]
        appkit::wake();
    }

    fn new_events(&mut self, event_loop: &ActiveEventLoop, _: native::StartCause) {
        if self.resumed {
            self.open_waiting(event_loop);
        }
    }

    fn resumed(&mut self, event_loop: &ActiveEventLoop) {
        self.resumed = true;
        self.everywhere(Event::Resumed);
        self.open_waiting(event_loop);
    }

    fn suspended(&mut self, _: &ActiveEventLoop) {
        self.everywhere(Event::Suspended);
    }

    fn memory_warning(&mut self, _: &ActiveEventLoop) {
        self.everywhere(Event::MemoryWarning);
    }

    fn about_to_wait(&mut self, event_loop: &ActiveEventLoop) {
        if self.resumed {
            self.open_waiting(event_loop);
        }
    }

    fn window_event(&mut self, _: &ActiveEventLoop, id: WindowId, event: WindowEvent) {
        if let Some(&handle) = self.ids.get(&id) {
            self.deliver(handle, event);
        }
    }

    fn device_event(&mut self, _: &ActiveEventLoop, id: DeviceId, event: native::DeviceEvent) {
        if self.listen == Listen::Never {
            return;
        }
        let event = Event::Device {
            device_id: self.device(id),
            event: device_event(event),
        };
        if let Some(open) = self
            .device_target()
            .and_then(|handle| self.windows.get_mut(handle))
        {
            open.events.push_back(event);
        }
    }
}

// -- events -----------------------------------------------------------------

// No fallback: a variant a newer winit adds fails to compile until it has
// an event here.
fn window_event(app: &mut App, event: WindowEvent) -> Event {
    match event {
        WindowEvent::ActivationTokenDone { serial, token } => {
            let serial = app.request(serial);
            app.requests.retain(|(_, id)| *id != serial);
            Event::ActivationTokenDone {
                serial,
                token: token.into_raw(),
            }
        }
        WindowEvent::Resized(size) => Event::Resized {
            width: i64::from(size.width),
            height: i64::from(size.height),
        },
        WindowEvent::Moved(position) => Event::Moved {
            x: position.x,
            y: position.y,
        },
        WindowEvent::CloseRequested => Event::Closed,
        WindowEvent::Destroyed => Event::Destroyed,
        WindowEvent::DroppedFile(path) => Event::DroppedFile {
            path: file_path(path),
        },
        WindowEvent::HoveredFile(path) => Event::HoveredFile {
            path: file_path(path),
        },
        WindowEvent::HoveredFileCancelled => Event::HoveredFileCancelled,
        WindowEvent::Focused(focused) => Event::Focused { focused },
        WindowEvent::KeyboardInput {
            device_id,
            event,
            is_synthetic,
        } => Event::KeyboardInput {
            device_id: app.device(device_id),
            event: key_event(event),
            is_synthetic,
        },
        WindowEvent::ModifiersChanged(modifiers) => Event::ModifiersChanged {
            modifiers: Modifiers::State {
                shift: modifiers.state().shift_key(),
                control: modifiers.state().control_key(),
                alt: modifiers.state().alt_key(),
                super_key: modifiers.state().super_key(),
                left_shift: side(modifiers.lshift_state()),
                right_shift: side(modifiers.rshift_state()),
                left_control: side(modifiers.lcontrol_state()),
                right_control: side(modifiers.rcontrol_state()),
                left_alt: side(modifiers.lalt_state()),
                right_alt: side(modifiers.ralt_state()),
                left_super: side(modifiers.lsuper_state()),
                right_super: side(modifiers.rsuper_state()),
            },
        },
        WindowEvent::Ime(ime) => Event::Ime {
            event: match ime {
                native::Ime::Enabled => Ime::Enabled,
                native::Ime::Preedit(text, cursor) => Ime::Preedit {
                    text,
                    cursor: cursor.map_or(CursorRange::None, |(start, end)| CursorRange::Range {
                        start: start as i64,
                        end: end as i64,
                    }),
                },
                native::Ime::Commit(text) => Ime::Commit { text },
                native::Ime::Disabled => Ime::Disabled,
            },
        },
        WindowEvent::CursorMoved {
            device_id,
            position,
        } => Event::CursorMoved {
            x: position.x,
            y: position.y,
            device_id: app.device(device_id),
        },
        WindowEvent::CursorEntered { device_id } => Event::CursorEntered {
            device_id: app.device(device_id),
        },
        WindowEvent::CursorLeft { device_id } => Event::CursorLeft {
            device_id: app.device(device_id),
        },
        WindowEvent::MouseWheel {
            device_id,
            delta,
            phase,
        } => Event::MouseWheel {
            delta: scroll(delta),
            phase: touch_phase(phase),
            device_id: app.device(device_id),
        },
        WindowEvent::MouseInput {
            device_id,
            state,
            button,
        } => Event::MouseInput {
            state: element_state(state),
            button: mouse_button(button),
            device_id: app.device(device_id),
        },
        WindowEvent::PinchGesture {
            device_id,
            delta,
            phase,
        } => Event::PinchGesture {
            device_id: app.device(device_id),
            delta,
            phase: touch_phase(phase),
        },
        WindowEvent::PanGesture {
            device_id,
            delta,
            phase,
        } => Event::PanGesture {
            device_id: app.device(device_id),
            x: f64::from(delta.x),
            y: f64::from(delta.y),
            phase: touch_phase(phase),
        },
        WindowEvent::DoubleTapGesture { device_id } => Event::DoubleTapGesture {
            device_id: app.device(device_id),
        },
        WindowEvent::RotationGesture {
            device_id,
            delta,
            phase,
        } => Event::RotationGesture {
            device_id: app.device(device_id),
            delta: f64::from(delta),
            phase: touch_phase(phase),
        },
        WindowEvent::TouchpadPressure {
            device_id,
            pressure,
            stage,
        } => Event::TouchpadPressure {
            device_id: app.device(device_id),
            pressure: f64::from(pressure),
            stage,
        },
        WindowEvent::AxisMotion {
            device_id,
            axis,
            value,
        } => Event::AxisMotion {
            device_id: app.device(device_id),
            axis: i64::from(axis),
            value,
        },
        WindowEvent::Touch(touch) => Event::Touch {
            device_id: app.device(touch.device_id),
            phase: touch_phase(touch.phase),
            x: touch.location.x,
            y: touch.location.y,
            force: match touch.force {
                None => TouchForce::None,
                Some(native::Force::Calibrated {
                    force,
                    max_possible_force,
                    altitude_angle,
                }) => TouchForce::Calibrated {
                    force,
                    max_possible_force,
                    altitude_angle: altitude_angle
                        .map_or(OptionalFloat::None, |value| OptionalFloat::Some { value }),
                },
                Some(native::Force::Normalized(force)) => TouchForce::Normalized { force },
            },
            // All 64 bits, as Haxe's Int64 keeps them.
            id: touch.id as i64,
        },
        WindowEvent::ScaleFactorChanged { scale_factor, .. } => {
            Event::ScaleFactorChanged { scale_factor }
        }
        WindowEvent::ThemeChanged(theme) => Event::ThemeChanged {
            theme: theme_of(theme),
        },
        WindowEvent::Occluded(occluded) => Event::Occluded { occluded },
        WindowEvent::RedrawRequested => Event::RedrawRequested,
    }
}

fn device_event(event: native::DeviceEvent) -> DeviceEvent {
    match event {
        native::DeviceEvent::Added => DeviceEvent::Added,
        native::DeviceEvent::Removed => DeviceEvent::Removed,
        native::DeviceEvent::MouseMotion { delta: (x, y) } => DeviceEvent::MouseMotion { x, y },
        native::DeviceEvent::MouseWheel { delta } => DeviceEvent::MouseWheel {
            delta: scroll(delta),
        },
        native::DeviceEvent::Motion { axis, value } => DeviceEvent::Motion {
            axis: i64::from(axis),
            value,
        },
        native::DeviceEvent::Button { button, state } => DeviceEvent::Button {
            button: i64::from(button),
            state: element_state(state),
        },
        native::DeviceEvent::Key(key) => DeviceEvent::Key {
            physical_key: physical_key(key.physical_key),
            state: element_state(key.state),
        },
    }
}

/// A path as Unicode when it is, otherwise its exact bytes.
fn file_path(path: std::path::PathBuf) -> FilePath {
    match path.into_os_string().into_string() {
        Ok(path) => FilePath::Utf8 { path },
        Err(path) => {
            #[cfg(unix)]
            {
                use std::os::unix::ffi::OsStringExt;
                FilePath::UnixBytes {
                    bytes: VariantBytes(path.into_vec()),
                }
            }
            #[cfg(windows)]
            {
                use std::os::windows::ffi::OsStrExt;
                FilePath::WindowsWide {
                    utf16le: VariantBytes(path.encode_wide().flat_map(u16::to_le_bytes).collect()),
                }
            }
            #[cfg(not(any(unix, windows)))]
            {
                FilePath::Utf8 {
                    path: path.to_string_lossy().into_owned(),
                }
            }
        }
    }
}

fn key_event(event: native::KeyEvent) -> KeyEvent {
    #[cfg(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    ))]
    let supplement = {
        use winit::platform::modifier_supplement::KeyEventExtModifierSupplement;
        KeySupplement::Supplement {
            key_without_modifiers: logical_key(&event.key_without_modifiers()),
            text_with_all_modifiers: optional_text(event.text_with_all_modifiers()),
        }
    };
    #[cfg(not(any(
        target_os = "windows",
        target_os = "macos",
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd",
        target_os = "redox"
    )))]
    let supplement = KeySupplement::Unavailable;
    KeyEvent::Input {
        physical_key: physical_key(event.physical_key),
        logical_key: logical_key(&event.logical_key),
        text: optional_text(event.text.as_deref()),
        location: match event.location {
            keys::KeyLocation::Standard => KeyLocation::Standard,
            keys::KeyLocation::Left => KeyLocation::Left,
            keys::KeyLocation::Right => KeyLocation::Right,
            keys::KeyLocation::Numpad => KeyLocation::Numpad,
        },
        state: element_state(event.state),
        repeat: event.repeat,
        supplement,
    }
}

fn optional_text(text: Option<&str>) -> OptionalText {
    text.map_or(OptionalText::None, |text| OptionalText::Some {
        text: text.to_owned(),
    })
}

fn side(state: keys::ModifiersKeyState) -> ModifiersKeyState {
    match state {
        keys::ModifiersKeyState::Pressed => ModifiersKeyState::Pressed,
        keys::ModifiersKeyState::Unknown => ModifiersKeyState::Unknown,
    }
}

fn element_state(state: native::ElementState) -> MouseElementState {
    match state {
        native::ElementState::Pressed => MouseElementState::Pressed,
        native::ElementState::Released => MouseElementState::Released,
    }
}

fn scroll(delta: native::MouseScrollDelta) -> MouseScrollDelta {
    match delta {
        native::MouseScrollDelta::LineDelta(x, y) => MouseScrollDelta::LineDelta {
            x: f64::from(x),
            y: f64::from(y),
        },
        native::MouseScrollDelta::PixelDelta(p) => MouseScrollDelta::PixelDelta { x: p.x, y: p.y },
    }
}

fn touch_phase(phase: native::TouchPhase) -> TouchPhase {
    match phase {
        native::TouchPhase::Started => TouchPhase::Started,
        native::TouchPhase::Moved => TouchPhase::Moved,
        native::TouchPhase::Ended => TouchPhase::Ended,
        native::TouchPhase::Cancelled => TouchPhase::Cancelled,
    }
}

fn mouse_button(button: native::MouseButton) -> MouseButton {
    match button {
        native::MouseButton::Left => MouseButton::Left,
        native::MouseButton::Right => MouseButton::Right,
        native::MouseButton::Middle => MouseButton::Middle,
        native::MouseButton::Back => MouseButton::Back,
        native::MouseButton::Forward => MouseButton::Forward,
        native::MouseButton::Other(button) => MouseButton::Other {
            button: i32::from(button),
        },
    }
}

fn theme_of(theme: windows::Theme) -> Theme {
    match theme {
        windows::Theme::Light => Theme::Light,
        windows::Theme::Dark => Theme::Dark,
    }
}

/// A size winit gives as `u32`, as the `Int` every runtime has.
fn clamp(value: u32) -> i32 {
    i32::try_from(value).unwrap_or(i32::MAX)
}

fn physical_key(key: keys::PhysicalKey) -> PhysicalKey {
    macro_rules! codes {
        ($($name:ident),* $(,)?) => {
            match key {
                $(keys::PhysicalKey::Code(keys::KeyCode::$name) => {
                    PhysicalKey::Code { code: KeyCode::$name }
                })*
                keys::PhysicalKey::Code(_) => PhysicalKey::Code { code: KeyCode::Unrecognized },
                keys::PhysicalKey::Unidentified(native) => PhysicalKey::Unidentified {
                    code: match native {
                        keys::NativeKeyCode::Unidentified => NativeKeyCode::Unidentified,
                        keys::NativeKeyCode::Android(code) => NativeKeyCode::Android { code: i64::from(code) },
                        keys::NativeKeyCode::MacOS(code) => NativeKeyCode::MacOS { code: i32::from(code) },
                        keys::NativeKeyCode::Windows(code) => NativeKeyCode::Windows { code: i32::from(code) },
                        keys::NativeKeyCode::Xkb(code) => NativeKeyCode::Xkb { code: i64::from(code) },
                    },
                },
            }
        };
    }
    xwindow_core::key_codes!(codes)
}

fn logical_key(key: &keys::Key) -> Key {
    macro_rules! named {
        ($($name:ident),* $(,)?) => {
            match key {
                $(keys::Key::Named(keys::NamedKey::$name) => Key::Named { key: NamedKey::$name },)*
                keys::Key::Named(_) => Key::Named { key: NamedKey::Unrecognized },
                keys::Key::Character(text) => Key::Character { text: text.to_string() },
                keys::Key::Dead(character) => Key::Dead {
                    character: character.map_or(OptionalText::None, |c| OptionalText::Some {
                        text: c.to_string(),
                    }),
                },
                keys::Key::Unidentified(native) => Key::Unidentified {
                    key: match native {
                        keys::NativeKey::Unidentified => NativeKey::Unidentified,
                        keys::NativeKey::Android(code) => NativeKey::Android { code: i64::from(*code) },
                        keys::NativeKey::MacOS(code) => NativeKey::MacOS { code: i32::from(*code) },
                        keys::NativeKey::Windows(code) => NativeKey::Windows { code: i32::from(*code) },
                        keys::NativeKey::Xkb(code) => NativeKey::Xkb { code: i64::from(*code) },
                        keys::NativeKey::Web(key) => NativeKey::Web { key: key.to_string() },
                    },
                },
            }
        };
    }
    xwindow_core::named_keys!(named)
}

fn cursor_icon(icon: CursorIcon) -> windows::CursorIcon {
    macro_rules! icons {
        ($($name:ident),* $(,)?) => {
            match icon {
                $(CursorIcon::$name => windows::CursorIcon::$name,)*
            }
        };
    }
    xwindow_core::cursor_icons!(icons)
}

// -- windows ----------------------------------------------------------------

fn attributes(a: &WindowAttributes) -> windows::WindowAttributes {
    let mut out = windows::Window::default_attributes();
    if let Some(title) = &a.title {
        out = out.with_title(title.get().as_str());
    }
    if a.width.is_some() || a.height.is_some() {
        let width = a.width.unwrap_or(800).max(0);
        let height = a.height.unwrap_or(600).max(0);
        out = out.with_inner_size(LogicalSize::new(width, height));
    }
    if a.minWidth.is_some() || a.minHeight.is_some() {
        out = out.with_min_inner_size(LogicalSize::new(
            a.minWidth.unwrap_or(0).max(0),
            a.minHeight.unwrap_or(0).max(0),
        ));
    }
    if a.maxWidth.is_some() || a.maxHeight.is_some() {
        out = out.with_max_inner_size(LogicalSize::new(
            a.maxWidth.unwrap_or(i32::MAX).max(0),
            a.maxHeight.unwrap_or(i32::MAX).max(0),
        ));
    }
    if a.x.is_some() || a.y.is_some() {
        out = out.with_position(LogicalPosition::new(a.x.unwrap_or(0), a.y.unwrap_or(0)));
    }
    if let Some(yes) = a.resizable {
        out = out.with_resizable(yes);
    }
    if let Some(yes) = a.maximized {
        out = out.with_maximized(yes);
    }
    if let Some(yes) = a.visible {
        out = out.with_visible(yes);
    }
    if let Some(yes) = a.decorations {
        out = out.with_decorations(yes);
    }
    if let Some(yes) = a.transparent {
        out = out.with_transparent(yes);
    }
    if let Some(yes) = a.blur {
        out = out.with_blur(yes);
    }
    if let Some(yes) = a.hasShadow {
        out = with_shadow(out, yes);
    }
    if let Some(yes) = a.contentProtected {
        out = out.with_content_protected(yes);
    }
    if let Some(yes) = a.fullscreen {
        out = out.with_fullscreen(yes.then_some(windows::Fullscreen::Borderless(None)));
    }
    if let Some(yes) = a.active {
        out = out.with_active(yes);
    }
    if let Some(level) = a.windowLevel.and_then(WindowLevel::from_native) {
        out = out.with_window_level(window_level(level));
    }
    if let Some(theme) = a.theme.and_then(Theme::from_native) {
        out = out.with_theme(Some(native_theme(theme)));
    }
    if let Some(rgba) = &a.icon {
        let rgba = rgba.get();
        match icon(
            unsafe { rgba.as_slice() },
            a.iconWidth.unwrap_or(0),
            a.iconHeight.unwrap_or(0),
        ) {
            Ok(icon) => out = out.with_window_icon(icon),
            Err(error) => host::raise(ErrorKind::Type, &error),
        }
    }
    if let Some(size) = limit(
        a.resizeIncrementWidth.unwrap_or(0),
        a.resizeIncrementHeight.unwrap_or(0),
    ) {
        out = out.with_resize_increments(size);
    }
    if a.closeButton.is_some() || a.minimizeButton.is_some() || a.maximizeButton.is_some() {
        out = out.with_enabled_buttons(buttons(
            a.closeButton.unwrap_or(true),
            a.minimizeButton.unwrap_or(true),
            a.maximizeButton.unwrap_or(true),
        ));
    }
    out
}

fn native_theme(theme: Theme) -> windows::Theme {
    match theme {
        Theme::Light => windows::Theme::Light,
        Theme::Dark => windows::Theme::Dark,
    }
}

/// An icon of `width` by `height` RGBA pixels, or none for no pixels.
fn icon(rgba: &[u8], width: i32, height: i32) -> Result<Option<windows::Icon>, String> {
    if rgba.is_empty() {
        return Ok(None);
    }
    let (Ok(width), Ok(height)) = (u32::try_from(width), u32::try_from(height)) else {
        return Err("window: an icon is width by height RGBA pixels".to_owned());
    };
    windows::Icon::from_rgba(rgba.to_vec(), width, height)
        .map(Some)
        .map_err(|error| format!("window: {error}"))
}

fn buttons(close: bool, minimize: bool, maximize: bool) -> windows::WindowButtons {
    let mut buttons = windows::WindowButtons::empty();
    buttons.set(windows::WindowButtons::CLOSE, close);
    buttons.set(windows::WindowButtons::MINIMIZE, minimize);
    buttons.set(windows::WindowButtons::MAXIMIZE, maximize);
    buttons
}

/// The system's shadow behind a window, where the platform draws one.
#[allow(unused_variables)]
fn with_shadow(out: windows::WindowAttributes, yes: bool) -> windows::WindowAttributes {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowAttributesExtMacOS;
        out.with_has_shadow(yes)
    }
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowAttributesExtWindows;
        out.with_undecorated_shadow(yes)
    }
    #[cfg(not(any(target_os = "macos", windows)))]
    out
}

fn window_level(level: WindowLevel) -> windows::WindowLevel {
    match level {
        WindowLevel::Normal => windows::WindowLevel::Normal,
        WindowLevel::AlwaysOnBottom => windows::WindowLevel::AlwaysOnBottom,
        WindowLevel::AlwaysOnTop => windows::WindowLevel::AlwaysOnTop,
    }
}

pub unsafe fn window_open(a: &WindowAttributes) -> i32 {
    let attributes = attributes(a);
    let sizing = a
        .scaleSizing
        .and_then(ScaleSizing::from_native)
        .unwrap_or_default();
    with(0, |l| {
        l.app.opening.push((attributes, sizing));
        if l.hosted() {
            if active(|event_loop| l.app.open_waiting(event_loop)).is_none() {
                l.app.opening.clear();
                host::raise(
                    ErrorKind::Runtime,
                    "window: under a host's event loop, a window opens in the program's turn",
                );
                return 0;
            }
        } else {
            // winit makes windows only while it runs, once the platform has
            // resumed, which on Android comes when the activity has a
            // surface; and on some platforms not in its first pass.
            let deadline = Instant::now() + Duration::from_secs(10);
            let mut passes = 0;
            while l.app.opened.is_empty() && passes < 16 && Instant::now() < deadline {
                if l.app.resumed {
                    passes += 1;
                    // A poll on macOS handles only what is queued, short of
                    // the pass of winit's loop that makes the window.
                    l.pump(Some(if cfg!(target_os = "macos") {
                        Duration::from_millis(1)
                    } else {
                        Duration::ZERO
                    }));
                } else {
                    l.pump(Some(Duration::from_millis(50)));
                }
            }
        }
        l.app.opening.clear();
        match l.app.opened.pop() {
            Some(Ok(handle)) => {
                // A radius is the window's own, so it is set once there is one.
                if let (Some(radius), Some(open)) = (a.blurRadius, l.app.windows.get(handle)) {
                    blur_radius(&open.window, radius);
                }
                handle
            }
            Some(Err(error)) => {
                host::raise(ErrorKind::Runtime, &format!("window: {error}"));
                0
            }
            None => {
                host::raise(ErrorKind::Runtime, "window: the platform made no window");
                0
            }
        }
    })
}

/// Taken when the loop next runs, so it never makes the loop itself.
pub unsafe fn window_listen_device_events(when: i32) {
    let listen = match DeviceEvents::from_native(when) {
        Some(DeviceEvents::Always) => Listen::Always,
        Some(DeviceEvents::Never) => Listen::Never,
        _ => Listen::WhenFocused,
    };
    LISTEN.set(Some(listen));
}

/// Runs `body` on the event loop if there is one, without making it.
fn made<T>(miss: T, body: impl FnOnce(&mut Loop) -> T) -> T {
    LOOP.with(|cell| match cell.try_borrow_mut() {
        Ok(mut slot) => match slot.as_mut() {
            Some(l) => body(l),
            None => miss,
        },
        Err(_) => miss,
    })
}

pub unsafe fn window_clipboard_text() -> OptionalText {
    made(None, |l| {
        let bytes = l.app.clipboard()?.read(xwindow_clipboard::TEXT)?;
        String::from_utf8(bytes).ok()
    })
    .map_or(OptionalText::None, |text| OptionalText::Some { text })
}

pub unsafe fn window_set_clipboard_text(text: Text) {
    let item = (
        xwindow_clipboard::TEXT.to_owned(),
        text.as_str().as_bytes().to_vec(),
    );
    made((), |l| {
        if let Some(clipboard) = l.app.clipboard() {
            clipboard.write(&[item]);
        }
    });
}

pub unsafe fn window_clipboard_type_count() -> i32 {
    made(0, |l| {
        let types = l.app.clipboard().map(|c| c.types()).unwrap_or_default();
        l.app.clipboard_types = types;
        l.app.clipboard_types.len() as i32
    })
}

pub unsafe fn window_clipboard_type(index: i32) -> Text {
    let name = made(None, |l| {
        usize::try_from(index)
            .ok()
            .and_then(|i| l.app.clipboard_types.get(i).cloned())
    });
    Text::new(&name.unwrap_or_default())
}

pub unsafe fn window_clipboard_data(mime: Text) -> OptionalBytes {
    let mime = mime.as_str().to_owned();
    made(None, |l| l.app.clipboard()?.read(&mime)).map_or(OptionalBytes::None, |bytes| {
        OptionalBytes::Some {
            bytes: VariantBytes(bytes),
        }
    })
}

thread_local! {
    /// Each `ClipboardItems`' types and bytes, until it is written.
    static ITEMS: RefCell<Slab<Vec<(String, Vec<u8>)>>> =
        const { RefCell::new(Slab::new(Kind::ClipboardItems)) };
}

pub unsafe fn clipboard_items_create() -> i32 {
    ITEMS.with(|items| items.borrow_mut().put(Vec::new()))
}

#[allow(unused_unsafe)]
pub unsafe fn clipboard_items_add(handle: i32, mime: Text, bytes: Buffer) {
    let item = (
        mime.as_str().to_owned(),
        unsafe { bytes.as_slice() }.to_vec(),
    );
    ITEMS.with(|items| {
        if let Some(list) = items.borrow_mut().get_mut(handle) {
            list.push(item);
        }
    });
}

pub unsafe fn clipboard_items_write(handle: i32) -> bool {
    let Some(list) = ITEMS.with(|items| items.borrow_mut().remove(handle)) else {
        return false;
    };
    made(false, |l| {
        l.app
            .clipboard()
            .is_some_and(|clipboard| clipboard.write(&list))
    })
}

pub unsafe fn window_valid(handle: i32) -> bool {
    window(handle, false, |_| true)
}

pub unsafe fn window_poll(handle: i32) -> Event {
    with(Event::None, |l| l.next(handle, None))
}

pub unsafe fn window_wait(handle: i32, timeout: f64) -> Event {
    let deadline = (timeout >= 0.0).then(|| {
        Instant::now() + Duration::try_from_secs_f64(timeout).unwrap_or(Duration::MAX / 2)
    });
    with(Event::None, |l| l.next(handle, Some(deadline)))
}

/// Thread-safe, coalesced application wake. Never touches the thread-local window state.
pub unsafe fn window_wake() -> bool {
    let Some(proxy) = WAKE_PROXY.get() else { return false; };
    if !WORK_PENDING.swap(true, Ordering::AcqRel) && proxy.send_event(()).is_err() {
        WORK_PENDING.store(false, Ordering::Release);
        return false;
    }
    true
}

pub unsafe fn window_close(handle: i32) {
    with((), |l| {
        if let Some(open) = l.app.windows.remove(handle) {
            l.app.ids.remove(&open.window.id());
            if l.app.focused == Some(handle) {
                l.app.focused = None;
            }
        }
    });
}

pub unsafe fn window_width(handle: i32) -> i32 {
    window(handle, 0, |w| clamp(w.inner_size().width))
}

pub unsafe fn window_height(handle: i32) -> i32 {
    window(handle, 0, |w| clamp(w.inner_size().height))
}

pub unsafe fn window_outer_width(handle: i32) -> i32 {
    window(handle, 0, |w| clamp(w.outer_size().width))
}

pub unsafe fn window_outer_height(handle: i32) -> i32 {
    window(handle, 0, |w| clamp(w.outer_size().height))
}

pub unsafe fn window_x(handle: i32) -> i32 {
    window(handle, 0, |w| w.outer_position().map_or(0, |p| p.x))
}

pub unsafe fn window_y(handle: i32) -> i32 {
    window(handle, 0, |w| w.outer_position().map_or(0, |p| p.y))
}

pub unsafe fn window_inner_x(handle: i32) -> i32 {
    window(handle, 0, |w| w.inner_position().map_or(0, |p| p.x))
}

pub unsafe fn window_inner_y(handle: i32) -> i32 {
    window(handle, 0, |w| w.inner_position().map_or(0, |p| p.y))
}

pub unsafe fn window_scale_factor(handle: i32) -> f64 {
    window(handle, 1.0, |w| w.scale_factor())
}

pub unsafe fn window_title(handle: i32) -> Text {
    let title = with(String::new(), |l| {
        l.app
            .windows
            .get(handle)
            .map(|open| open.title.clone())
            .unwrap_or_default()
    });
    Text::new(&title)
}

pub unsafe fn window_has_focus(handle: i32) -> bool {
    window(handle, false, |w| w.has_focus())
}

pub unsafe fn window_is_visible(handle: i32) -> bool {
    window(handle, false, |w| w.is_visible().unwrap_or(true))
}

pub unsafe fn window_is_minimized(handle: i32) -> bool {
    window(handle, false, |w| w.is_minimized().unwrap_or(false))
}

pub unsafe fn window_is_maximized(handle: i32) -> bool {
    window(handle, false, |w| w.is_maximized())
}

pub unsafe fn window_is_fullscreen(handle: i32) -> bool {
    window(handle, false, |w| w.fullscreen().is_some())
}

pub unsafe fn window_is_resizable(handle: i32) -> bool {
    window(handle, false, |w| w.is_resizable())
}

pub unsafe fn window_is_decorated(handle: i32) -> bool {
    window(handle, false, |w| w.is_decorated())
}

pub unsafe fn window_theme(handle: i32) -> i32 {
    window(handle, Theme::Light, |w| {
        w.theme().map_or(Theme::Light, theme_of)
    })
    .native()
}

fn split(window: &windows::Window) -> (i32, [i64; 4]) {
    match (window.window_handle(), window.display_handle()) {
        (Ok(w), Ok(d)) => xwindow_core::raw::split(w.as_raw(), d.as_raw()),
        _ => (xwindow_core::raw::NONE, [0; 4]),
    }
}

pub unsafe fn window_platform(handle: i32) -> i32 {
    window(handle, xwindow_core::raw::NONE, |w| split(w).0)
}

pub unsafe fn window_raw(handle: i32, which: i32) -> i64 {
    window(handle, 0, |w| {
        usize::try_from(which)
            .ok()
            .and_then(|i| split(w).1.get(i).copied())
            .unwrap_or(0)
    })
}

pub unsafe fn window_set_title(handle: i32, title: Text) {
    with((), |l| {
        if let Some(open) = l.app.windows.get_mut(handle) {
            open.window.set_title(title.as_str());
            open.title = title.as_str().to_owned();
        }
    });
}

/// Runs `body` on window `handle`. AppKit tells winit what a call changed
/// within the call, outside a pump, where winit drops it for want of a
/// handler; so on macOS xwindow delivers the change itself.
fn changing(handle: i32, body: impl FnOnce(&mut Open)) {
    with((), |l| {
        #[cfg(target_os = "macos")]
        let before = l.app.states();
        if let Some(open) = l.app.windows.get_mut(handle) {
            body(open);
        }
        #[cfg(target_os = "macos")]
        l.app.changed(before);
    });
}

pub unsafe fn window_set_size(handle: i32, width: i32, height: i32) {
    changing(handle, |open| {
        // A size applied at once comes back here, with no Resized.
        let size = LogicalSize::new(width.max(0), height.max(0));
        if let Some(size) = open.window.request_inner_size(size) {
            open.events.push_back(Event::Resized {
                width: i64::from(size.width),
                height: i64::from(size.height),
            });
        }
    });
}

/// A logical size, or none for zero by zero.
fn limit(width: i32, height: i32) -> Option<LogicalSize<i32>> {
    (width > 0 || height > 0).then(|| LogicalSize::new(width.max(0), height.max(0)))
}

pub unsafe fn window_set_min_size(handle: i32, width: i32, height: i32) {
    changing(handle, |open| {
        open.window.set_min_inner_size(limit(width, height))
    });
}

pub unsafe fn window_set_max_size(handle: i32, width: i32, height: i32) {
    changing(handle, |open| {
        open.window.set_max_inner_size(limit(width, height))
    });
}

pub unsafe fn window_set_position(handle: i32, x: i32, y: i32) {
    changing(handle, |open| {
        open.window.set_outer_position(LogicalPosition::new(x, y))
    });
}

pub unsafe fn window_set_resizable(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_resizable(yes));
}

pub unsafe fn window_set_minimized(handle: i32, yes: bool) {
    changing(handle, |open| open.window.set_minimized(yes));
}

pub unsafe fn window_set_maximized(handle: i32, yes: bool) {
    changing(handle, |open| open.window.set_maximized(yes));
}

pub unsafe fn window_set_fullscreen(handle: i32, yes: bool) {
    changing(handle, |open| {
        open.window
            .set_fullscreen(yes.then_some(windows::Fullscreen::Borderless(None)))
    });
}

pub unsafe fn window_set_decorations(handle: i32, yes: bool) {
    changing(handle, |open| open.window.set_decorations(yes));
}

pub unsafe fn window_set_visible(handle: i32, yes: bool) {
    changing(handle, |open| open.window.set_visible(yes));
}

pub unsafe fn window_set_window_level(handle: i32, level: i32) {
    if let Some(level) = WindowLevel::from_native(level) {
        window(handle, (), |w| w.set_window_level(window_level(level)));
    }
}

pub unsafe fn window_set_transparent(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_transparent(yes));
}

pub unsafe fn window_set_blur(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_blur(yes));
}

/// Tells the runtime this thread is outside its heap until dropped.
struct Blocking;

impl Blocking {
    fn new() -> Self {
        host::blocking(true);
        Blocking
    }
}

impl Drop for Blocking {
    fn drop(&mut self) {
        host::blocking(false);
    }
}

/// What the macOS pump needs from AppKit beyond winit.
#[cfg(target_os = "macos")]
mod appkit {
    use std::cell::Cell;
    use std::ffi::{c_char, c_void};
    use std::time::{Duration, Instant};

    type Id = *mut c_void;
    type Sel = *const c_void;

    #[link(name = "objc")]
    unsafe extern "C" {
        fn sel_registerName(name: *const c_char) -> Sel;
        fn objc_getClass(name: *const c_char) -> Id;
        fn objc_msgSend();
        fn objc_autoreleasePoolPush() -> *mut c_void;
        fn objc_autoreleasePoolPop(pool: *mut c_void);
    }

    #[link(name = "System")]
    unsafe extern "C" {
        static _NSConcreteGlobalBlock: [usize; 0];
    }

    #[link(name = "Foundation", kind = "framework")]
    unsafe extern "C" {
        static NSDefaultRunLoopMode: Id;
    }

    /// `objc_msgSend`, typed as the method it sends.
    unsafe fn send<F: Copy>() -> F {
        let send: unsafe extern "C" fn() = objc_msgSend;
        unsafe { std::mem::transmute_copy(&send) }
    }

    unsafe fn class(name: &std::ffi::CStr) -> Id {
        unsafe { objc_getClass(name.as_ptr()) }
    }

    unsafe fn sel(name: &std::ffi::CStr) -> Sel {
        unsafe { sel_registerName(name.as_ptr()) }
    }

    const LEFT_MOUSE_DOWN: u64 = 1;
    const APPLICATION_DEFINED: u64 = 15;
    /// The subtype marking xwindow's serve events among application-defined
    /// ones; winit's own stop events have subtype 0.
    const SERVE: i16 = 0x7877;
    const WAKE: i16 = 0x7878;

    /// How long the run being served may wait for an event.
    #[derive(Clone, Copy)]
    enum Until {
        Now,
        At(Instant),
        Never,
    }

    thread_local! {
        /// The last serve event posted; an earlier one still queued is spent.
        static POSTED: Cell<isize> = const { Cell::new(0) };
        static UNTIL: Cell<Until> = const { Cell::new(Until::Now) };
        static WATCHING: Cell<bool> = const { Cell::new(false) };
        /// The last left button press, retained.
        static PRESSED: Cell<Id> = const { Cell::new(std::ptr::null_mut()) };
    }

    #[repr(C)]
    struct Descriptor {
        reserved: usize,
        size: usize,
    }

    #[repr(C)]
    struct Block {
        isa: *const c_void,
        flags: i32,
        reserved: i32,
        invoke: unsafe extern "C" fn(*mut Block, Id) -> Id,
        descriptor: *const Descriptor,
    }

    static DESCRIPTOR: Descriptor = Descriptor {
        reserved: 0,
        size: std::mem::size_of::<Block>(),
    };

    /// Makes the next run of the application serve events itself for up to
    /// `timeout`, forever for none, and then stop: true unless the
    /// application has yet to finish launching, which winit's launch stops.
    ///
    /// The run starts on a serve event posted now, which AppKit sees before
    /// it waits, and a local event monitor does the serving: it takes
    /// events with a deadline and hands each to the application, which
    /// returns at the deadline, where a run waits for an event that stops
    /// it and AppKit can miss one posted while it sleeps.
    pub fn serve(timeout: Option<Duration>) -> bool {
        unsafe {
            if !WATCHING.get() {
                // Launch completion is monotonic. Querying it on every idle
                // poll fetches dynamic properties through synchronous
                // LaunchServices IPC; check only until the monitor is installed.
                let running: Id = send::<unsafe extern "C" fn(Id, Sel) -> Id>()(
                    class(c"NSRunningApplication"),
                    sel(c"currentApplication"),
                );
                let launched = send::<unsafe extern "C" fn(Id, Sel) -> bool>()(
                    running,
                    sel(c"isFinishedLaunching"),
                );
                if !launched {
                    return false;
                }
                WATCHING.set(true);
                // A global block: it captures nothing and is never freed.
                let block = Box::leak(Box::new(Block {
                    isa: _NSConcreteGlobalBlock.as_ptr().cast(),
                    flags: 1 << 28,
                    reserved: 0,
                    invoke: seen,
                    descriptor: &DESCRIPTOR,
                }));
                send::<unsafe extern "C" fn(Id, Sel, u64, *mut Block) -> Id>()(
                    class(c"NSEvent"),
                    sel(c"addLocalMonitorForEventsMatchingMask:handler:"),
                    (1 << APPLICATION_DEFINED) | (1 << LEFT_MOUSE_DOWN),
                    block,
                );
            }
            UNTIL.set(match timeout {
                Some(timeout) if timeout.is_zero() => Until::Now,
                Some(timeout) => Until::At(Instant::now() + timeout),
                None => Until::Never,
            });
            let posted = POSTED.get() + 1;
            POSTED.set(posted);
            post(posted, SERVE);
            true
        }
    }

    /// Starts AppKit's move of the window `view` is in, with the pointer, as
    /// from the last left button press: winit uses the current event, which
    /// the press no longer is once the program handles it.
    pub fn drag(view: *mut c_void) -> bool {
        unsafe {
            let pressed = PRESSED.get();
            if pressed.is_null() {
                return false;
            }
            let window = send::<unsafe extern "C" fn(Id, Sel) -> Id>()(view, sel(c"window"));
            if window.is_null() {
                return false;
            }
            send::<unsafe extern "C" fn(Id, Sel, Id)>()(
                window,
                sel(c"performWindowDragWithEvent:"),
                pressed,
            );
            true
        }
    }

    /// Return a waiting external pump without manufacturing a window event.
    pub fn wake() {
        unsafe { post(0, WAKE); }
    }

    /// Posts an event behind the queued events; the queue retains it.
    unsafe fn post(posted: isize, subtype: i16) {
        #[repr(C)]
        struct Point(f64, f64);
        unsafe {
            let pool = objc_autoreleasePoolPush();
            let event = send::<
                unsafe extern "C" fn(Id, Sel, u64, Point, u64, f64, isize, Id, i16, isize, isize) -> Id,
            >()(
                class(c"NSEvent"),
                sel(c"otherEventWithType:location:modifierFlags:timestamp:windowNumber:context:subtype:data1:data2:"),
                APPLICATION_DEFINED,
                Point(0.0, 0.0),
                0,
                0.0,
                0,
                std::ptr::null_mut(),
                subtype,
                posted,
                0,
            );
            send::<unsafe extern "C" fn(Id, Sel, Id, bool)>()(
                application(),
                sel(c"postEvent:atStart:"),
                event,
                false,
            );
            objc_autoreleasePoolPop(pool);
        }
    }

    unsafe fn application() -> Id {
        unsafe {
            send::<unsafe extern "C" fn(Id, Sel) -> Id>()(
                class(c"NSApplication"),
                sel(c"sharedApplication"),
            )
        }
    }

    /// The next event, waiting for one until `until`; none once it passes.
    unsafe fn next(until: Until) -> Id {
        unsafe {
            let date = match until {
                Until::Now => send::<unsafe extern "C" fn(Id, Sel) -> Id>()(
                    class(c"NSDate"),
                    sel(c"distantPast"),
                ),
                Until::At(at) => send::<unsafe extern "C" fn(Id, Sel, f64) -> Id>()(
                    class(c"NSDate"),
                    sel(c"dateWithTimeIntervalSinceNow:"),
                    at.saturating_duration_since(Instant::now()).as_secs_f64(),
                ),
                Until::Never => send::<unsafe extern "C" fn(Id, Sel) -> Id>()(
                    class(c"NSDate"),
                    sel(c"distantFuture"),
                ),
            };
            send::<unsafe extern "C" fn(Id, Sel, u64, Id, Id, bool) -> Id>()(
                application(),
                sel(c"nextEventMatchingMask:untilDate:inMode:dequeue:"),
                u64::MAX,
                date,
                NSDefaultRunLoopMode,
                true,
            )
        }
    }

    /// The monitor. On the last serve event posted it hands the application
    /// what is queued and what arrives until the deadline, returning once
    /// it has handled anything, then stops the application. It drops every
    /// serve event and winit's stop events, and leaves others alone.
    unsafe extern "C" fn seen(_: *mut Block, event: Id) -> Id {
        unsafe {
            let kind = send::<unsafe extern "C" fn(Id, Sel) -> u64>()(event, sel(c"type"));
            if kind == LEFT_MOUSE_DOWN {
                let retained = send::<unsafe extern "C" fn(Id, Sel) -> Id>()(event, sel(c"retain"));
                let last = PRESSED.replace(retained);
                if !last.is_null() {
                    send::<unsafe extern "C" fn(Id, Sel)>()(last, sel(c"release"));
                }
                return event;
            }
            let subtype = send::<unsafe extern "C" fn(Id, Sel) -> i16>()(event, sel(c"subtype"));
            if subtype == WAKE {
                return std::ptr::null_mut();
            }
            if subtype != SERVE {
                return event;
            }
            let data = send::<unsafe extern "C" fn(Id, Sel) -> isize>()(event, sel(c"data1"));
            if data != POSTED.get() {
                return std::ptr::null_mut();
            }
            let app = application();
            let mut until = UNTIL.get();
            loop {
                let pool = objc_autoreleasePoolPush();
                let next = next(until);
                let more = !next.is_null();
                if more {
                    let kind = send::<unsafe extern "C" fn(Id, Sel) -> u64>()(next, sel(c"type"));
                    let ours = kind == APPLICATION_DEFINED && {
                        let subtype =
                            send::<unsafe extern "C" fn(Id, Sel) -> i16>()(next, sel(c"subtype"));
                        if subtype == WAKE {
                            until = Until::Now;
                        }
                        subtype == SERVE || subtype == WAKE || subtype == 0
                    };
                    if !ours {
                        send::<unsafe extern "C" fn(Id, Sel, Id)>()(app, sel(c"sendEvent:"), next);
                        until = Until::Now;
                    }
                }
                objc_autoreleasePoolPop(pool);
                if !more {
                    break;
                }
            }
            send::<unsafe extern "C" fn(Id, Sel, Id)>()(app, sel(c"stop:"), std::ptr::null_mut());
            std::ptr::null_mut()
        }
    }
}

/// The background blur behind a transparent window: a radius on macOS,
/// through the private call winit makes for `set_blur` with a radius of 80,
/// and the platform's blur on or off elsewhere.
fn blur_radius(window: &windows::Window, radius: i32) {
    #[cfg(target_os = "macos")]
    {
        use std::ffi::{c_char, c_void};

        #[link(name = "objc")]
        unsafe extern "C" {
            fn sel_registerName(name: *const c_char) -> *const c_void;
            fn objc_msgSend();
        }
        #[link(name = "CoreGraphics", kind = "framework")]
        unsafe extern "C" {
            fn CGSMainConnectionID() -> *mut c_void;
            fn CGSSetWindowBackgroundBlurRadius(
                connection: *mut c_void,
                window: isize,
                radius: i64,
            ) -> i32;
        }
        let Ok(handle) = window.window_handle() else {
            return;
        };
        let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw() else {
            return;
        };
        unsafe {
            // [[view window] windowNumber], each send typed as its method is.
            let object: unsafe extern "C" fn(*mut c_void, *const c_void) -> *mut c_void =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let integer: unsafe extern "C" fn(*mut c_void, *const c_void) -> isize =
                std::mem::transmute(objc_msgSend as unsafe extern "C" fn());
            let ns_window = object(
                appkit.ns_view.as_ptr(),
                sel_registerName(c"window".as_ptr()),
            );
            if ns_window.is_null() {
                return;
            }
            let number = integer(ns_window, sel_registerName(c"windowNumber".as_ptr()));
            CGSSetWindowBackgroundBlurRadius(
                CGSMainConnectionID(),
                number,
                i64::from(radius.max(0)),
            );
        }
    }
    #[cfg(not(target_os = "macos"))]
    window.set_blur(radius > 0);
}

pub unsafe fn window_set_blur_radius(handle: i32, radius: i32) {
    window(handle, (), |w| blur_radius(w, radius));
}

#[allow(unused_variables)]
pub unsafe fn window_set_has_shadow(handle: i32, yes: bool) {
    #[cfg(target_os = "macos")]
    {
        use winit::platform::macos::WindowExtMacOS;
        window(handle, (), |w| w.set_has_shadow(yes));
    }
    #[cfg(windows)]
    {
        use winit::platform::windows::WindowExtWindows;
        window(handle, (), |w| w.set_undecorated_shadow(yes));
    }
}

pub unsafe fn window_set_content_protected(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_content_protected(yes));
}

pub unsafe fn window_set_scale_sizing(handle: i32, sizing: i32) {
    with((), |l| {
        if let (Some(open), Some(sizing)) = (
            l.app.windows.get_mut(handle),
            ScaleSizing::from_native(sizing),
        ) {
            open.sizing = sizing;
        }
    });
}

pub unsafe fn window_set_theme(handle: i32, theme: Option<i32>) {
    let theme = theme.and_then(Theme::from_native).map(native_theme);
    window(handle, (), |w| w.set_theme(theme));
}

#[allow(unused_unsafe)]
pub unsafe fn window_set_icon(handle: i32, rgba: Buffer, width: i32, height: i32) {
    match icon(unsafe { rgba.as_slice() }, width, height) {
        Ok(icon) => window(handle, (), |w| w.set_window_icon(icon)),
        Err(error) => host::raise(ErrorKind::Type, &error),
    }
}

pub unsafe fn window_set_resize_increments(handle: i32, width: i32, height: i32) {
    window(handle, (), |w| {
        w.set_resize_increments(limit(width, height))
    });
}

pub unsafe fn window_set_enabled_buttons(handle: i32, close: bool, minimize: bool, maximize: bool) {
    window(handle, (), |w| {
        w.set_enabled_buttons(buttons(close, minimize, maximize))
    });
}

pub unsafe fn window_set_exclusive_fullscreen(handle: i32, mode: i32) {
    let Some(mode) = video_mode(mode, None, |m| Some(m.clone())) else {
        return;
    };
    window(handle, (), |w| {
        w.set_fullscreen(Some(windows::Fullscreen::Exclusive(mode)))
    });
}

pub unsafe fn window_drag(handle: i32) -> bool {
    window(handle, false, |w| {
        #[cfg(target_os = "macos")]
        if let Ok(handle) = w.window_handle()
            && let raw_window_handle::RawWindowHandle::AppKit(appkit) = handle.as_raw()
        {
            return appkit::drag(appkit.ns_view.as_ptr());
        }
        w.drag_window().is_ok()
    })
}

pub unsafe fn window_drag_resize(handle: i32, direction: i32) -> bool {
    let Some(direction) = ResizeDirection::from_native(direction) else {
        return false;
    };
    let direction = match direction {
        ResizeDirection::East => windows::ResizeDirection::East,
        ResizeDirection::North => windows::ResizeDirection::North,
        ResizeDirection::NorthEast => windows::ResizeDirection::NorthEast,
        ResizeDirection::NorthWest => windows::ResizeDirection::NorthWest,
        ResizeDirection::South => windows::ResizeDirection::South,
        ResizeDirection::SouthEast => windows::ResizeDirection::SouthEast,
        ResizeDirection::SouthWest => windows::ResizeDirection::SouthWest,
        ResizeDirection::West => windows::ResizeDirection::West,
    };
    window(handle, false, |w| w.drag_resize_window(direction).is_ok())
}

pub unsafe fn window_show_menu(handle: i32, x: f64, y: f64) {
    window(handle, (), |w| {
        w.show_window_menu(LogicalPosition::new(x, y))
    });
}

/// On macOS xwindow delivers the redraw itself: winit delivers one only on a
/// pump that waits.
pub unsafe fn window_request_redraw(handle: i32) {
    #[cfg(target_os = "macos")]
    with((), |l| {
        if let Some(open) = l.app.windows.get_mut(handle) {
            if open.redraw == Redraw::None {
                open.redraw = Redraw::Asked;
            }
        }
    });
    #[cfg(not(target_os = "macos"))]
    window(handle, (), |w| w.request_redraw());
}

pub unsafe fn window_pre_present_notify(handle: i32) {
    window(handle, (), |w| w.pre_present_notify());
}

pub unsafe fn window_focus(handle: i32) {
    changing(handle, |open| open.window.focus_window());
}

pub unsafe fn window_request_attention(handle: i32, attention: i32) {
    let request = match Attention::from_native(attention) {
        Some(Attention::Informational) => Some(windows::UserAttentionType::Informational),
        Some(Attention::Critical) => Some(windows::UserAttentionType::Critical),
        _ => None,
    };
    window(handle, (), |w| w.request_user_attention(request));
}

pub unsafe fn window_set_cursor_icon(handle: i32, icon: i32) {
    let Some(icon) = CursorIcon::from_native(icon) else {
        return;
    };
    window(handle, (), |w| w.set_cursor(cursor_icon(icon)));
}

#[allow(unused_unsafe)]
pub unsafe fn window_set_cursor_image(
    handle: i32,
    rgba: Buffer,
    width: i32,
    height: i32,
    hot_x: i32,
    hot_y: i32,
) {
    let (Ok(width), Ok(height), Ok(hot_x), Ok(hot_y)) = (
        u16::try_from(width),
        u16::try_from(height),
        u16::try_from(hot_x),
        u16::try_from(hot_y),
    ) else {
        return host::raise(
            ErrorKind::Type,
            "window: a cursor is at most 65535 pixels across",
        );
    };
    let pixels = unsafe { rgba.as_slice() }.to_vec();
    let source = match windows::CustomCursor::from_rgba(pixels, width, height, hot_x, hot_y) {
        Ok(source) => source,
        Err(error) => return host::raise(ErrorKind::Type, &format!("window: {error}")),
    };
    with((), |l| {
        let cursor = match &l.driver {
            #[cfg(not(target_os = "ios"))]
            Driver::Pumped(events) => Some(events.create_custom_cursor(source)),
            Driver::Hosted => active(|event_loop| event_loop.create_custom_cursor(source)),
        };
        match (cursor, l.app.windows.get(handle)) {
            (Some(cursor), Some(open)) => open.window.set_cursor(cursor),
            (None, _) => host::raise(
                ErrorKind::Runtime,
                "window: under a host's event loop, a cursor is made in the program's turn",
            ),
            _ => {}
        }
    });
}

pub unsafe fn window_set_cursor_visible(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_cursor_visible(yes));
}

pub unsafe fn window_set_cursor_grab(handle: i32, grab: i32) -> bool {
    let mode = match CursorGrab::from_native(grab) {
        Some(CursorGrab::Confined) => windows::CursorGrabMode::Confined,
        Some(CursorGrab::Locked) => windows::CursorGrabMode::Locked,
        _ => windows::CursorGrabMode::None,
    };
    window(handle, false, |w| w.set_cursor_grab(mode).is_ok())
}

/// Moving the cursor brings no CursorMoved on macOS, so xwindow delivers one.
pub unsafe fn window_set_cursor_position(handle: i32, x: f64, y: f64) -> bool {
    with(false, |l| {
        let Some(open) = l.app.windows.get(handle) else {
            return false;
        };
        let position = LogicalPosition::new(x, y);
        if open.window.set_cursor_position(position).is_err() {
            return false;
        }
        #[cfg(target_os = "macos")]
        {
            let position = position.to_physical(open.window.scale_factor());
            // AppKit reports every pointer as this one device.
            let device_id = DeviceId::dummy();
            l.app.deliver(
                handle,
                WindowEvent::CursorMoved {
                    device_id,
                    position,
                },
            );
        }
        true
    })
}

pub unsafe fn window_set_cursor_hittest(handle: i32, yes: bool) -> bool {
    window(handle, false, |w| w.set_cursor_hittest(yes).is_ok())
}

pub unsafe fn window_set_ime_purpose(handle: i32, purpose: i32) {
    let purpose = match ImePurpose::from_native(purpose) {
        Some(ImePurpose::Password) => windows::ImePurpose::Password,
        Some(ImePurpose::Terminal) => windows::ImePurpose::Terminal,
        _ => windows::ImePurpose::Normal,
    };
    window(handle, (), |w| w.set_ime_purpose(purpose));
}

pub unsafe fn window_set_ime_allowed(handle: i32, yes: bool) {
    window(handle, (), |w| w.set_ime_allowed(yes));
}

pub unsafe fn window_set_ime_cursor_area(handle: i32, x: f64, y: f64, width: f64, height: f64) {
    window(handle, (), |w| {
        w.set_ime_cursor_area(
            LogicalPosition::new(x, y),
            LogicalSize::new(width.max(0.0), height.max(0.0)),
        )
    });
}

pub unsafe fn window_request_activation_token(handle: i32) -> i64 {
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    ))]
    {
        use winit::platform::startup_notify::WindowExtStartupNotify;
        with(0, |l| {
            let serial = l
                .app
                .windows
                .get(handle)
                .and_then(|open| open.window.request_activation_token().ok());
            serial.map_or(0, |serial| l.app.request(serial))
        })
    }
    #[cfg(not(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "dragonfly",
        target_os = "netbsd",
        target_os = "openbsd"
    )))]
    {
        let _ = handle;
        0
    }
}

// -- monitors ---------------------------------------------------------------

/// Monitors are kept for the program's life, each once, so the same display
/// is the same handle. A program sees a handful.
fn monitor_handle(index: usize) -> i32 {
    kept_handle(Kind::Monitor, index)
}

/// A handle for what is kept for the program's life: the kind, and the
/// index from one.
fn kept_handle(kind: Kind, index: usize) -> i32 {
    ((kind as i32) << 26) | (index as i32 + 1)
}

/// The index of a kept handle of `kind`.
fn kept_index(kind: Kind, handle: i32) -> Option<usize> {
    (handle >> 26 == kind as i32)
        .then(|| (handle & ((1 << 26) - 1)) as usize)
        .and_then(|i| i.checked_sub(1))
}

fn monitor<T>(handle: i32, miss: T, body: impl FnOnce(&MonitorHandle) -> T) -> T {
    let index = kept_index(Kind::Monitor, handle);
    let mut body = Some(body);
    let found = with(None, |l| {
        index
            .and_then(|i| l.app.monitors.get(i))
            .map(|m| (body.take().expect("called once"))(m))
    });
    found.unwrap_or(miss)
}

pub unsafe fn window_current_monitor(handle: i32) -> i32 {
    with(0, |l| {
        let found = l
            .app
            .windows
            .get(handle)
            .and_then(|o| o.window.current_monitor());
        l.app.monitor(found)
    })
}

pub unsafe fn window_primary_monitor(handle: i32) -> i32 {
    with(0, |l| {
        let found = l
            .app
            .windows
            .get(handle)
            .and_then(|o| o.window.primary_monitor());
        l.app.monitor(found)
    })
}

pub unsafe fn window_monitor_count(handle: i32) -> i32 {
    window(handle, 0, |w| w.available_monitors().count() as i32)
}

pub unsafe fn window_monitor(handle: i32, index: i32) -> i32 {
    with(0, |l| {
        let found = l.app.windows.get(handle).and_then(|o| {
            usize::try_from(index)
                .ok()
                .and_then(|i| o.window.available_monitors().nth(i))
        });
        l.app.monitor(found)
    })
}

pub unsafe fn monitor_valid(handle: i32) -> bool {
    monitor(handle, false, |_| true)
}

pub unsafe fn monitor_name(handle: i32) -> Text {
    Text::new(&monitor(handle, String::new(), |m| {
        m.name().unwrap_or_default()
    }))
}

pub unsafe fn monitor_width(handle: i32) -> i32 {
    monitor(handle, 0, |m| clamp(m.size().width))
}

pub unsafe fn monitor_height(handle: i32) -> i32 {
    monitor(handle, 0, |m| clamp(m.size().height))
}

pub unsafe fn monitor_x(handle: i32) -> i32 {
    monitor(handle, 0, |m| m.position().x)
}

pub unsafe fn monitor_y(handle: i32) -> i32 {
    monitor(handle, 0, |m| m.position().y)
}

pub unsafe fn monitor_scale_factor(handle: i32) -> f64 {
    monitor(handle, 1.0, |m| m.scale_factor())
}

pub unsafe fn monitor_refresh_rate(handle: i32) -> i32 {
    monitor(handle, 0, |m| {
        m.refresh_rate_millihertz().map_or(0, |r| clamp(r))
    })
}

pub unsafe fn monitor_video_mode_count(handle: i32) -> i32 {
    monitor(handle, 0, |m| m.video_modes().count() as i32)
}

pub unsafe fn monitor_video_mode(handle: i32, index: i32) -> i32 {
    let found = monitor(handle, None, |m| {
        usize::try_from(index)
            .ok()
            .and_then(|i| m.video_modes().nth(i))
    });
    with(0, |l| l.app.video_mode(found))
}

// -- video modes --------------------------------------------------------------

fn video_mode<T>(handle: i32, miss: T, body: impl FnOnce(&VideoModeHandle) -> T) -> T {
    let index = kept_index(Kind::VideoMode, handle);
    let mut body = Some(body);
    let found = with(None, |l| {
        index
            .and_then(|i| l.app.video_modes.get(i))
            .map(|m| (body.take().expect("called once"))(m))
    });
    found.unwrap_or(miss)
}

pub unsafe fn video_mode_valid(handle: i32) -> bool {
    video_mode(handle, false, |_| true)
}

pub unsafe fn video_mode_width(handle: i32) -> i32 {
    video_mode(handle, 0, |m| clamp(m.size().width))
}

pub unsafe fn video_mode_height(handle: i32) -> i32 {
    video_mode(handle, 0, |m| clamp(m.size().height))
}

pub unsafe fn video_mode_bit_depth(handle: i32) -> i32 {
    video_mode(handle, 0, |m| i32::from(m.bit_depth()))
}

pub unsafe fn video_mode_refresh_rate(handle: i32) -> i32 {
    video_mode(handle, 0, |m| clamp(m.refresh_rate_millihertz()))
}

pub unsafe fn video_mode_monitor(handle: i32) -> i32 {
    let found = video_mode(handle, None, |m| Some(m.monitor()));
    with(0, |l| l.app.monitor(found))
}
