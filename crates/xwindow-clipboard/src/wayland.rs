//! Wayland: a data device on the window's own connection, served from a
//! worker thread with its own event queue and loop. The worker answers the
//! compositor while the window thread waits on it, so a program can read the
//! selection it set itself.

use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::io::{ErrorKind, Read, Write};
use std::os::fd::AsFd;
use std::ptr::NonNull;
use std::rc::Rc;
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use calloop::channel::{self, Channel, Sender};
use calloop::timer::{TimeoutAction, Timer};
use calloop::{EventLoop, LoopHandle, PostAction, RegistrationToken};
use sctk::data_device_manager::data_device::{DataDevice, DataDeviceHandler};
use sctk::data_device_manager::data_offer::{DataOfferHandler, DragOffer};
use sctk::data_device_manager::data_source::{CopyPasteSource, DataSourceHandler};
use sctk::data_device_manager::{DataDeviceManagerState, WritePipe};
use sctk::reexports::calloop_wayland_source::WaylandSource;
use sctk::reexports::client::globals::registry_queue_init;
use sctk::reexports::client::protocol::wl_callback::{self, WlCallback};
use sctk::reexports::client::protocol::wl_data_device::WlDataDevice;
use sctk::reexports::client::protocol::wl_data_device_manager::DndAction;
use sctk::reexports::client::protocol::wl_data_source::WlDataSource;
use sctk::reexports::client::protocol::wl_keyboard::{self, WlKeyboard};
use sctk::reexports::client::protocol::wl_pointer::{self, WlPointer};
use sctk::reexports::client::protocol::wl_seat::WlSeat;
use sctk::reexports::client::protocol::wl_surface::WlSurface;
use sctk::reexports::client::{Connection, Dispatch, Proxy, QueueHandle};
use sctk::registry::{ProvidesRegistryState, RegistryState};
use sctk::seat::{Capability, SeatHandler, SeatState};
use sctk::{delegate_data_device, delegate_registry, delegate_seat, registry_handlers};
use smithay_client_toolkit as sctk;
use wayland_backend::client::ObjectId;
use wayland_backend::sys::client::Backend;

/// How long a read waits for the selection's owner to send all its bytes.
const TIMEOUT: Duration = Duration::from_secs(1);

/// The names clients give UTF-8 text, best first. `text/plain` stands for
/// all of them: a read takes the best offered, a write offers every one.
const TEXT_NAMES: [&str; 5] = [
    "text/plain;charset=utf-8",
    "UTF8_STRING",
    "text/plain",
    "TEXT",
    "STRING",
];

fn is_text(mime: &str) -> bool {
    crate::essence(mime) == crate::TEXT || TEXT_NAMES.iter().any(|n| n.eq_ignore_ascii_case(mime))
}

/// The type xwindow reports for an offered one.
fn name(mime: &str) -> String {
    if is_text(mime) {
        crate::TEXT.to_owned()
    } else {
        crate::essence(mime)
    }
}

pub struct Clipboard {
    /// Dropping it closes the channel, which ends the worker.
    jobs: Option<Sender<Job>>,
    worker: Option<JoinHandle<()>>,
}

impl Clipboard {
    pub unsafe fn connect(display: NonNull<c_void>) -> Result<Self, String> {
        let backend = unsafe { Backend::from_foreign_display(display.as_ptr().cast()) };
        let connection = Connection::from_backend(backend);
        let (jobs, channel) = channel::channel();
        let (ready, started) = mpsc::channel();
        let worker = std::thread::Builder::new()
            .name("xwindow-clipboard".to_owned())
            .spawn(move || serve(connection, channel, ready))
            .map_err(|e| format!("clipboard: {e}"))?;
        match started.recv() {
            Ok(Ok(())) => Ok(Self {
                jobs: Some(jobs),
                worker: Some(worker),
            }),
            Ok(Err(e)) => Err(e),
            Err(_) => Err("clipboard: the worker stopped".to_owned()),
        }
    }

    pub fn types(&self) -> Vec<String> {
        self.ask(Job::Types).unwrap_or_default()
    }

    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        self.ask(|reply| Job::Read(mime.to_owned(), reply))
            .flatten()
    }

    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        self.ask(|reply| Job::Write(items.to_vec(), reply))
            .unwrap_or(false)
    }

    /// Hands the worker a job and waits for its answer. The worker gives up
    /// on a read after `TIMEOUT`; the wait outlasts that.
    fn ask<T>(&self, job: impl FnOnce(mpsc::Sender<T>) -> Job) -> Option<T> {
        let (reply, answer) = mpsc::channel();
        self.jobs.as_ref()?.send(job(reply)).ok()?;
        answer.recv_timeout(2 * TIMEOUT).ok()
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        self.jobs = None;
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

/// Work for the worker, each done after a roundtrip so that it sees every
/// event the compositor sent before it was asked: the latest serial, the
/// latest selection.
enum Job {
    Types(mpsc::Sender<Vec<String>>),
    Read(String, mpsc::Sender<Option<Vec<u8>>>),
    Write(Vec<(String, Vec<u8>)>, mpsc::Sender<bool>),
    /// Whether the compositor kept the source a write set as the selection.
    Kept(WlDataSource, mpsc::Sender<bool>),
}

fn serve(connection: Connection, jobs: Channel<Job>, ready: mpsc::Sender<Result<(), String>>) {
    let (mut event_loop, mut state) = match start(connection, jobs) {
        Ok(started) => started,
        Err(e) => {
            let _ = ready.send(Err(e));
            return;
        }
    };
    let _ = ready.send(Ok(()));
    while !state.closed && event_loop.dispatch(None, &mut state).is_ok() {}
}

fn start(
    connection: Connection,
    jobs: Channel<Job>,
) -> Result<(EventLoop<'static, State>, State), String> {
    let failed = |e: &dyn std::fmt::Display| format!("clipboard: {e}");
    let (globals, queue) = registry_queue_init::<State>(&connection).map_err(|e| failed(&e))?;
    let qh = queue.handle();
    let manager = DataDeviceManagerState::bind(&globals, &qh).map_err(|e| failed(&e))?;
    let event_loop = EventLoop::try_new().map_err(|e| failed(&e))?;
    let handle = event_loop.handle();
    handle
        .insert_source(jobs, |event, _, state: &mut State| match event {
            channel::Event::Msg(job) => state.after_roundtrip(job),
            channel::Event::Closed => state.closed = true,
        })
        .map_err(|e| failed(&e.error))?;
    WaylandSource::new(connection.clone(), queue)
        .insert(handle.clone())
        .map_err(|e| failed(&e.error))?;
    let mut state = State {
        connection,
        registry: RegistryState::new(&globals),
        seat_state: SeatState::new(&globals, &qh),
        qh,
        handle,
        manager,
        seats: HashMap::new(),
        serial: None,
        sources: Vec::new(),
        jobs: VecDeque::new(),
        reading: None,
        closed: false,
    };
    for seat in state.seat_state.seats() {
        state.seat(&seat);
    }
    // Sent now, before the window can be shown and focused: mutter sends the
    // selection only to the devices a client had when it gained focus.
    let _ = state.connection.flush();
    Ok((event_loop, state))
}

/// The types a source offers, each with its bytes.
type Offer = Vec<(String, Rc<[u8]>)>;

struct State {
    connection: Connection,
    qh: QueueHandle<State>,
    handle: LoopHandle<'static, State>,
    registry: RegistryState,
    seat_state: SeatState,
    manager: DataDeviceManagerState,
    seats: HashMap<ObjectId, Seat>,
    /// The seat and serial of the latest input event, which a selection is
    /// set with.
    serial: Option<(ObjectId, u32)>,
    /// Each source offered and not yet cancelled, with its types' bytes.
    sources: Vec<(CopyPasteSource, Offer)>,
    /// Jobs waiting for their roundtrips, which complete in order.
    jobs: VecDeque<Job>,
    reading: Option<Reading>,
    closed: bool,
}

/// A read in progress: the bytes so far, until the pipe ends or the timer
/// fires, whichever is first.
struct Reading {
    bytes: Vec<u8>,
    reply: mpsc::Sender<Option<Vec<u8>>>,
    pipe: RegistrationToken,
    timer: RegistrationToken,
}

/// What ended a read; its own source removes itself.
enum End {
    Pipe,
    Timer,
    Superseded,
}

/// The device, keyboard and pointer of one seat. The keyboard and pointer
/// are only for their serials.
struct Seat {
    device: DataDevice,
    keyboard: Option<WlKeyboard>,
    pointer: Option<WlPointer>,
}

impl Drop for Seat {
    fn drop(&mut self) {
        release(self.keyboard.take(), self.pointer.take());
    }
}

fn release(keyboard: Option<WlKeyboard>, pointer: Option<WlPointer>) {
    if let Some(keyboard) = keyboard.filter(|k| k.version() >= 3) {
        keyboard.release();
    }
    if let Some(pointer) = pointer.filter(|p| p.version() >= 3) {
        pointer.release();
    }
}

fn nonblocking(fd: impl AsFd) -> std::io::Result<()> {
    let flags = rustix::fs::fcntl_getfl(&fd)?;
    Ok(rustix::fs::fcntl_setfl(
        &fd,
        flags | rustix::fs::OFlags::NONBLOCK,
    )?)
}

impl State {
    fn seat(&mut self, seat: &WlSeat) -> &mut Seat {
        self.seats.entry(seat.id()).or_insert_with(|| Seat {
            device: self.manager.get_data_device(&self.qh, seat),
            keyboard: None,
            pointer: None,
        })
    }

    /// The device of the seat that last had input, else of any seat.
    fn device(&self) -> Option<&DataDevice> {
        let latest = self
            .serial
            .as_ref()
            .and_then(|(seat, _)| self.seats.get(seat));
        latest
            .or_else(|| self.seats.values().next())
            .map(|seat| &seat.device)
    }

    /// The types the selection offers, as its owner names them.
    fn offered(&self) -> Vec<String> {
        self.device()
            .and_then(|device| device.data().selection_offer())
            .map(|offer| offer.with_mime_types(<[String]>::to_vec))
            .unwrap_or_default()
    }

    fn after_roundtrip(&mut self, job: Job) {
        self.jobs.push_back(job);
        self.connection.display().sync(&self.qh, ());
        let _ = self.connection.flush();
    }

    fn run(&mut self, job: Job) {
        match job {
            Job::Types(reply) => {
                let mut types = Vec::new();
                for name in self.offered().iter().map(|mime| name(mime)) {
                    if !types.contains(&name) {
                        types.push(name);
                    }
                }
                let _ = reply.send(types);
            }
            Job::Read(mime, reply) => self.read(&mime, reply),
            Job::Write(items, reply) => match self.write(items) {
                Some(source) => self.after_roundtrip(Job::Kept(source, reply)),
                None => {
                    let _ = reply.send(false);
                }
            },
            Job::Kept(source, reply) => {
                let _ = reply.send(self.sources.iter().any(|(s, _)| *s.inner() == source));
            }
        }
    }

    /// Offers `items` as the selection; the source it set.
    fn write(&mut self, items: Vec<(String, Vec<u8>)>) -> Option<WlDataSource> {
        let serial = self.serial.as_ref().map_or(0, |(_, serial)| *serial);
        let mut offered = Vec::new();
        for (mime, _) in &items {
            if mime == crate::TEXT {
                offered.extend(TEXT_NAMES.iter().map(|n| n.to_string()));
            } else {
                offered.push(mime.clone());
            }
        }
        // A compositor may ignore a selection set with a serial no newer than
        // the selection's (mutter does), and with no input between two writes
        // the serial is the same. Withdrawing ours first leaves none to beat.
        self.sources.clear();
        let device = self.device()?;
        let source = self.manager.create_copy_paste_source(&self.qh, offered);
        source.set_selection(device, serial);
        let _ = self.connection.flush();
        let set = source.inner().clone();
        let items = items.into_iter().map(|(m, b)| (m, Rc::from(b))).collect();
        self.sources.push((source, items));
        Some(set)
    }

    /// Asks the selection's owner for `mime`, and reads what it sends from
    /// the loop, which also serves the selection when it is this program's.
    fn read(&mut self, mime: &str, reply: mpsc::Sender<Option<Vec<u8>>>) {
        self.end_read(false, End::Superseded);
        let offered = self.offered();
        let pick = if mime == crate::TEXT {
            TEXT_NAMES
                .iter()
                .find_map(|n| offered.iter().find(|m| m.eq_ignore_ascii_case(n)))
                .or_else(|| offered.iter().find(|m| is_text(m)))
        } else {
            offered.iter().find(|m| crate::essence(m) == mime)
        };
        let offer = self
            .device()
            .and_then(|device| device.data().selection_offer());
        let pipe = pick
            .zip(offer)
            .and_then(|(pick, offer)| offer.receive(pick.clone()).ok())
            .filter(|pipe| nonblocking(pipe).is_ok());
        let Some(pipe) = pipe else {
            let _ = reply.send(None);
            return;
        };
        let _ = self.connection.flush();

        let mut chunk = vec![0; 1 << 16];
        let pipe = self.handle.insert_source(pipe, move |_, file, state| {
            loop {
                match unsafe { file.get_mut() }.read(&mut chunk) {
                    Ok(0) => break state.end_read(true, End::Pipe),
                    Ok(n) => {
                        if let Some(reading) = state.reading.as_mut() {
                            reading.bytes.extend_from_slice(&chunk[..n]);
                        }
                    }
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => break PostAction::Continue,
                    Err(_) => break state.end_read(false, End::Pipe),
                }
            }
        });
        let timer = self
            .handle
            .insert_source(Timer::from_duration(TIMEOUT), |_, _, state| {
                state.end_read(false, End::Timer);
                TimeoutAction::Drop
            });
        match (pipe, timer) {
            (Ok(pipe), Ok(timer)) => {
                self.reading = Some(Reading {
                    bytes: Vec::new(),
                    reply,
                    pipe,
                    timer,
                })
            }
            (pipe, timer) => {
                for token in [pipe.ok(), timer.ok()].into_iter().flatten() {
                    self.handle.remove(token);
                }
                let _ = reply.send(None);
            }
        }
    }

    /// Answers the read in progress, with its bytes when `whole`, and
    /// removes its sources but the one that ended it.
    fn end_read(&mut self, whole: bool, by: End) -> PostAction {
        if let Some(reading) = self.reading.take() {
            if !matches!(by, End::Pipe) {
                self.handle.remove(reading.pipe);
            }
            if !matches!(by, End::Timer) {
                self.handle.remove(reading.timer);
            }
            let _ = reading.reply.send(whole.then_some(reading.bytes));
        }
        PostAction::Remove
    }
}

impl DataSourceHandler for State {
    /// Writes the bytes for `mime` from the loop, without blocking it: the
    /// reader may be this program's own loop.
    fn send_request(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        source: &WlDataSource,
        mime: String,
        pipe: WritePipe,
    ) {
        let name = name(&mime);
        let bytes = self
            .sources
            .iter()
            .find(|(s, _)| s.inner() == source)
            .and_then(|(_, items)| items.iter().find(|(m, _)| *m == name))
            .map(|(_, bytes)| bytes.clone());
        // Dropping the pipe closes it, which tells the reader there is none.
        let Some(bytes) = bytes.filter(|_| nonblocking(&pipe).is_ok()) else {
            return;
        };
        let mut sent = 0;
        let _ = self.handle.insert_source(pipe, move |_, file, _| {
            let file = unsafe { file.get_mut() };
            while sent < bytes.len() {
                match file.write(&bytes[sent..]) {
                    Ok(n) => sent += n,
                    Err(e) if e.kind() == ErrorKind::Interrupted => {}
                    Err(e) if e.kind() == ErrorKind::WouldBlock => return PostAction::Continue,
                    Err(_) => break,
                }
            }
            PostAction::Remove
        });
    }

    fn cancelled(&mut self, _: &Connection, _: &QueueHandle<Self>, source: &WlDataSource) {
        self.sources.retain(|(s, _)| s.inner() != source);
    }

    fn accept_mime(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataSource,
        _: Option<String>,
    ) {
    }
    fn dnd_dropped(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn dnd_finished(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource) {}
    fn action(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataSource, _: DndAction) {}
}

// The selection is read when asked; drag and drop is not this crate's.
impl DataDeviceHandler for State {
    fn enter(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &WlDataDevice,
        _: f64,
        _: f64,
        _: &WlSurface,
    ) {
    }
    fn leave(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn motion(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice, _: f64, _: f64) {}
    fn selection(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
    fn drop_performed(&mut self, _: &Connection, _: &QueueHandle<Self>, _: &WlDataDevice) {}
}

impl DataOfferHandler for State {
    fn source_actions(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
    fn selected_action(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        _: &mut DragOffer,
        _: DndAction,
    ) {
    }
}

impl SeatHandler for State {
    fn seat_state(&mut self) -> &mut SeatState {
        &mut self.seat_state
    }

    fn new_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.seat(&seat);
    }

    fn new_capability(
        &mut self,
        _: &Connection,
        qh: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        let id = seat.id();
        let entry = self.seat(&seat);
        match capability {
            Capability::Keyboard if entry.keyboard.is_none() => {
                entry.keyboard = Some(seat.get_keyboard(qh, id));
            }
            Capability::Pointer if entry.pointer.is_none() => {
                entry.pointer = Some(seat.get_pointer(qh, id));
            }
            _ => {}
        }
    }

    fn remove_capability(
        &mut self,
        _: &Connection,
        _: &QueueHandle<Self>,
        seat: WlSeat,
        capability: Capability,
    ) {
        if let Some(entry) = self.seats.get_mut(&seat.id()) {
            match capability {
                Capability::Keyboard => release(entry.keyboard.take(), None),
                Capability::Pointer => release(None, entry.pointer.take()),
                _ => {}
            }
        }
    }

    fn remove_seat(&mut self, _: &Connection, _: &QueueHandle<Self>, seat: WlSeat) {
        self.seats.remove(&seat.id());
        if self.serial.as_ref().is_some_and(|(id, _)| *id == seat.id()) {
            self.serial = None;
        }
    }
}

/// Keyboard focus and keys carry serials a selection can be set with.
impl Dispatch<WlKeyboard, ObjectId> for State {
    fn event(
        state: &mut Self,
        _: &WlKeyboard,
        event: wl_keyboard::Event,
        seat: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_keyboard::Event::Enter { serial, .. } | wl_keyboard::Event::Key { serial, .. } =
            event
        {
            state.serial = Some((seat.clone(), serial));
        }
    }
}

/// As do pointer entries and buttons.
impl Dispatch<WlPointer, ObjectId> for State {
    fn event(
        state: &mut Self,
        _: &WlPointer,
        event: wl_pointer::Event,
        seat: &ObjectId,
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_pointer::Event::Enter { serial, .. } | wl_pointer::Event::Button { serial, .. } =
            event
        {
            state.serial = Some((seat.clone(), serial));
        }
    }
}

/// A roundtrip's end: the job that waited on it runs.
impl Dispatch<WlCallback, ()> for State {
    fn event(
        state: &mut Self,
        _: &WlCallback,
        event: wl_callback::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        if let wl_callback::Event::Done { .. } = event
            && let Some(job) = state.jobs.pop_front()
        {
            state.run(job);
        }
    }
}

impl ProvidesRegistryState for State {
    registry_handlers![SeatState];

    fn registry(&mut self) -> &mut RegistryState {
        &mut self.registry
    }
}

delegate_seat!(State);
delegate_data_device!(State);
delegate_registry!(State);
