//! The X11 CLIPBOARD selection, through a connection of its own.
//!
//! A hidden window owns the selection while it holds what `write` gave it,
//! and a thread answers other clients' requests for it. That thread is the
//! connection's only reader of events: it serves requests itself and hands
//! the replies to our own requests to the caller over a channel, so reading
//! a selection we own works too.

use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use x11rb::connection::{Connection, RequestConnection as _};
use x11rb::protocol::Event;
use x11rb::protocol::xproto::{
    Atom, AtomEnum, ChangeWindowAttributesAux, ClientMessageEvent, ConnectionExt as _,
    CreateWindowAux, EventMask, PropMode, Property, SELECTION_NOTIFY_EVENT, SelectionNotifyEvent,
    SelectionRequestEvent, Timestamp, Window, WindowClass,
};
use x11rb::rust_connection::RustConnection;
use x11rb::wrapper::ConnectionExt as _;
use x11rb::{COPY_DEPTH_FROM_PARENT, CURRENT_TIME, NONE};

x11rb::atom_manager! {
    Atoms: AtomsCookie {
        CLIPBOARD,
        TARGETS,
        TIMESTAMP,
        INCR,
        UTF8_STRING,
        TEXT,
        TEXT_PLAIN: b"text/plain",
        TEXT_PLAIN_UTF8: b"text/plain;charset=utf-8",
        // The property on our window that conversions arrive in.
        XWINDOW_SELECTION,
        // Appended to, empty, to learn the server's time.
        XWINDOW_STAMP,
        // Sent to our own window to stop the thread.
        XWINDOW_QUIT,
    }
}

/// How long a reply, or each piece of an incremental one, may take.
const PATIENCE: Duration = Duration::from_secs(1);
/// How long an incremental send may wait for its reader before it is dropped.
const ABANDONED: Duration = Duration::from_secs(10);

pub struct Clipboard {
    conn: Arc<RustConnection>,
    window: Window,
    atoms: Atoms,
    offer: Arc<Mutex<Option<Offer>>>,
    /// Events on our window: replies to our conversions and property changes.
    events: Receiver<Event>,
    thread: Option<JoinHandle<()>>,
}

/// What we serve while we own the selection.
struct Offer {
    time: Timestamp,
    /// Each target, the type its property is written with, and its bytes.
    targets: Vec<(Atom, Atom, Arc<[u8]>)>,
}

/// A send too large for one request, a piece each time the reader deletes
/// the last.
struct Incremental {
    window: Window,
    property: Atom,
    type_: Atom,
    data: Arc<[u8]>,
    sent: usize,
    last: Instant,
}

enum Conversion {
    Got {
        type_: Atom,
        format: u8,
        value: Vec<u8>,
    },
    Refused,
    /// The owner did not answer in time; asking it again would wait as long.
    Silent,
}

impl Clipboard {
    pub fn connect() -> Result<Self, String> {
        let fail = |e: &dyn std::fmt::Display| format!("clipboard: {e}");
        let (conn, screen) = RustConnection::connect(None).map_err(|e| fail(&e))?;
        let root = conn.setup().roots[screen].root;
        let atoms = Atoms::new(&conn).map_err(|e| fail(&e))?;
        let atoms = atoms.reply().map_err(|e| fail(&e))?;
        let window = conn.generate_id().map_err(|e| fail(&e))?;
        conn.create_window(
            COPY_DEPTH_FROM_PARENT,
            window,
            root,
            -1,
            -1,
            1,
            1,
            0,
            WindowClass::INPUT_ONLY,
            x11rb::COPY_FROM_PARENT,
            &CreateWindowAux::new().event_mask(EventMask::PROPERTY_CHANGE),
        )
        .map_err(|e| fail(&e))?
        .check()
        .map_err(|e| fail(&e))?;
        // Room for a property's bytes in one ChangeProperty, with its header
        // and a big request's longer length.
        let chunk = conn.maximum_request_bytes().saturating_sub(32) & !3;

        let conn = Arc::new(conn);
        let offer = Arc::new(Mutex::new(None));
        let (sender, events) = mpsc::channel();
        let thread = {
            let (conn, offer) = (conn.clone(), offer.clone());
            std::thread::Builder::new()
                .name("xwindow-clipboard".into())
                .spawn(move || serve(&conn, window, atoms, &offer, &sender, chunk))
                .map_err(|e| fail(&e))?
        };
        Ok(Self {
            conn,
            window,
            atoms,
            offer,
            events,
            thread: Some(thread),
        })
    }

    pub fn types(&self) -> Vec<String> {
        let Ok(targets) = self.targets() else {
            return Vec::new();
        };
        let mut types = Vec::new();
        for (_, name) in self.named(&targets) {
            let mime = match name.as_str() {
                "UTF8_STRING" | "STRING" | "TEXT" => crate::TEXT.to_owned(),
                name if name.contains('/') => crate::essence(name),
                _ => continue,
            };
            if !types.contains(&mime) {
                types.push(mime);
            }
        }
        types
    }

    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        // Without TARGETS, ask for the likely names anyway.
        let targets = match self.targets() {
            Ok(targets) => Some(targets),
            Err(Conversion::Silent) => return None,
            Err(_) => None,
        };
        let a = &self.atoms;
        let candidates: Vec<Atom> = if mime == crate::TEXT {
            let text = [
                a.UTF8_STRING,
                a.TEXT_PLAIN_UTF8,
                a.TEXT_PLAIN,
                AtomEnum::STRING.into(),
                a.TEXT,
            ];
            text.into_iter()
                .filter(|t| targets.as_ref().is_none_or(|ts| ts.contains(t)))
                .collect()
        } else if let Some(targets) = &targets {
            self.named(targets)
                .into_iter()
                .filter(|(_, name)| crate::essence(name) == mime)
                .map(|(atom, _)| atom)
                .collect()
        } else {
            vec![self.intern(mime)?]
        };
        for target in candidates {
            match self.convert(target) {
                Conversion::Got { type_, value, .. } if mime == crate::TEXT => {
                    if let Some(text) = self.decode(target, type_, value) {
                        return Some(text);
                    }
                }
                Conversion::Got { value, .. } => return Some(value),
                Conversion::Refused => {}
                Conversion::Silent => return None,
            }
        }
        None
    }

    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        let a = self.atoms;
        let mut targets: Vec<(Atom, Atom, Arc<[u8]>)> = Vec::new();
        for (mime, bytes) in items {
            let data: Arc<[u8]> = bytes.as_slice().into();
            let served: Vec<(Atom, Atom, Arc<[u8]>)> = if mime == crate::TEXT {
                let latin1: Vec<u8> = String::from_utf8_lossy(bytes)
                    .chars()
                    .map(|c| u8::try_from(c).unwrap_or(b'?'))
                    .collect();
                vec![
                    (a.UTF8_STRING, a.UTF8_STRING, data.clone()),
                    (a.TEXT_PLAIN_UTF8, a.TEXT_PLAIN_UTF8, data.clone()),
                    (a.TEXT_PLAIN, a.TEXT_PLAIN, data.clone()),
                    (
                        AtomEnum::STRING.into(),
                        AtomEnum::STRING.into(),
                        latin1.into(),
                    ),
                    (a.TEXT, a.UTF8_STRING, data),
                ]
            } else {
                let Some(atom) = self.intern(mime) else {
                    return false;
                };
                vec![(atom, atom, data)]
            };
            for target in served {
                if !targets.iter().any(|t| t.0 == target.0) {
                    targets.push(target);
                }
            }
        }
        let time = self.now();
        // Held across taking ownership, so that the thread, losing an
        // earlier ownership, cannot drop this offer.
        let mut offer = self.offer.lock().unwrap();
        *offer = Some(Offer { time, targets });
        let owner = self
            .conn
            .set_selection_owner(self.window, a.CLIPBOARD, time)
            .ok()
            .and_then(|_| self.conn.get_selection_owner(a.CLIPBOARD).ok())
            .and_then(|c| c.reply().ok());
        let owned = owner.is_some_and(|o| o.owner == self.window);
        if !owned {
            *offer = None;
        }
        owned
    }

    /// The selection's targets.
    fn targets(&self) -> Result<Vec<Atom>, Conversion> {
        match self.convert(self.atoms.TARGETS) {
            Conversion::Got {
                format: 32, value, ..
            } => Ok(value
                .as_chunks::<4>()
                .0
                .iter()
                .map(|b| u32::from_ne_bytes(*b))
                .collect()),
            Conversion::Got { .. } => Err(Conversion::Refused),
            other => Err(other),
        }
    }

    /// Each atom with its name; the requests go out together.
    fn named(&self, atoms: &[Atom]) -> Vec<(Atom, String)> {
        let cookies: Vec<_> = atoms
            .iter()
            .map(|&atom| (atom, self.conn.get_atom_name(atom)))
            .collect();
        cookies
            .into_iter()
            .filter_map(|(atom, cookie)| {
                let name = cookie.ok()?.reply().ok()?.name;
                Some((atom, String::from_utf8(name).ok()?))
            })
            .collect()
    }

    fn intern(&self, name: &str) -> Option<Atom> {
        let cookie = self.conn.intern_atom(false, name.as_bytes()).ok()?;
        Some(cookie.reply().ok()?.atom)
    }

    /// Text as UTF-8, from a target's reply.
    fn decode(&self, target: Atom, type_: Atom, value: Vec<u8>) -> Option<Vec<u8>> {
        let a = &self.atoms;
        let latin1 =
            |v: Vec<u8>| -> Vec<u8> { v.into_iter().map(char::from).collect::<String>().into() };
        if type_ == AtomEnum::STRING.into() {
            Some(latin1(value))
        } else if type_ == a.UTF8_STRING
            || target == a.TEXT_PLAIN_UTF8
            || std::str::from_utf8(&value).is_ok()
        {
            Some(value)
        } else if target == a.TEXT_PLAIN {
            Some(latin1(value))
        } else {
            // Some other encoding, as TEXT may answer.
            None
        }
    }

    /// The next of our window's events that `matches`, within `PATIENCE`.
    fn expect(&self, matches: impl Fn(&Event) -> bool) -> Option<Event> {
        let deadline = Instant::now() + PATIENCE;
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            match self.events.recv_timeout(left) {
                Ok(event) if matches(&event) => return Some(event),
                Ok(_) => {}
                Err(RecvTimeoutError::Timeout | RecvTimeoutError::Disconnected) => return None,
            }
        }
    }

    /// Asks the owner for `target` and waits for it, piece by piece when
    /// the owner sends it incrementally.
    fn convert(&self, target: Atom) -> Conversion {
        let (conn, a, window) = (&*self.conn, &self.atoms, self.window);
        // Leftovers from a conversion that gave up.
        while self.events.try_recv().is_ok() {}
        let sent = conn
            .convert_selection(
                window,
                a.CLIPBOARD,
                target,
                a.XWINDOW_SELECTION,
                CURRENT_TIME,
            )
            .and_then(|_| conn.flush());
        if sent.is_err() {
            return Conversion::Refused;
        }
        let notified = self.expect(|e| {
            matches!(e, Event::SelectionNotify(n) if n.selection == a.CLIPBOARD && n.target == target)
        });
        let Some(Event::SelectionNotify(notify)) = notified else {
            return Conversion::Silent;
        };
        if notify.property == NONE {
            return Conversion::Refused;
        }
        // Deleting the property is what asks an incremental owner for more.
        let take = || {
            conn.get_property(
                true,
                window,
                a.XWINDOW_SELECTION,
                AtomEnum::ANY,
                0,
                u32::MAX / 4,
            )
            .ok()?
            .reply()
            .ok()
        };
        let Some(reply) = take() else {
            return Conversion::Refused;
        };
        if reply.type_ != a.INCR {
            return Conversion::Got {
                type_: reply.type_,
                format: reply.format,
                value: reply.value,
            };
        }
        let mut value = Vec::new();
        loop {
            let piece = self.expect(|e| {
                matches!(e, Event::PropertyNotify(p)
                    if p.atom == a.XWINDOW_SELECTION && p.state == Property::NEW_VALUE)
            });
            if piece.is_none() {
                return Conversion::Silent;
            }
            let Some(reply) = take() else {
                return Conversion::Refused;
            };
            if reply.value.is_empty() {
                return Conversion::Got {
                    type_: reply.type_,
                    format: reply.format,
                    value,
                };
            }
            value.extend_from_slice(&reply.value);
        }
    }

    /// The server's time, which an append to our own property reports.
    fn now(&self) -> Timestamp {
        while self.events.try_recv().is_ok() {}
        let a = &self.atoms;
        let sent = self
            .conn
            .change_property8(
                PropMode::APPEND,
                self.window,
                a.XWINDOW_STAMP,
                AtomEnum::INTEGER,
                &[],
            )
            .and_then(|_| self.conn.flush());
        let stamped = sent.ok().and_then(|_| {
            self.expect(|e| matches!(e, Event::PropertyNotify(p) if p.atom == a.XWINDOW_STAMP))
        });
        match stamped {
            Some(Event::PropertyNotify(p)) => p.time,
            _ => CURRENT_TIME,
        }
    }
}

impl Drop for Clipboard {
    fn drop(&mut self) {
        let quit = ClientMessageEvent::new(32, self.window, self.atoms.XWINDOW_QUIT, [0u32; 5]);
        let sent = self
            .conn
            .send_event(false, self.window, EventMask::NO_EVENT, quit)
            .and_then(|_| self.conn.flush());
        if let (Ok(()), Some(thread)) = (sent, self.thread.take()) {
            let _ = thread.join();
        }
        let _ = self.conn.destroy_window(self.window);
        let _ = self.conn.flush();
    }
}

/// The thread: answers requests for what we own, and passes our window's
/// other events to the caller.
fn serve(
    conn: &RustConnection,
    window: Window,
    atoms: Atoms,
    offer: &Mutex<Option<Offer>>,
    events: &Sender<Event>,
    chunk: usize,
) {
    let mut sends: Vec<Incremental> = Vec::new();
    while let Ok(event) = conn.wait_for_event() {
        sends.retain(|s| s.last.elapsed() < ABANDONED);
        match event {
            Event::SelectionRequest(request) => {
                let offer = offer.lock().unwrap();
                answer(conn, &atoms, offer.as_ref(), &request, &mut sends, chunk);
            }
            Event::SelectionClear(clear) if clear.selection == atoms.CLIPBOARD => {
                let mut offer = offer.lock().unwrap();
                let owner = conn
                    .get_selection_owner(atoms.CLIPBOARD)
                    .ok()
                    .and_then(|c| c.reply().ok());
                if owner.is_none_or(|o| o.owner != window) {
                    *offer = None;
                }
            }
            Event::PropertyNotify(p)
                if p.state == Property::DELETE
                    && sends
                        .iter()
                        .any(|s| (s.window, s.property) == (p.window, p.atom)) =>
            {
                let i = sends
                    .iter()
                    .position(|s| (s.window, s.property) == (p.window, p.atom))
                    .unwrap();
                let send = &mut sends[i];
                // The piece after the last is empty, which ends the send.
                let end = send.data.len().min(send.sent + chunk);
                let piece = &send.data[send.sent..end];
                let _ = conn.change_property8(
                    PropMode::REPLACE,
                    send.window,
                    send.property,
                    send.type_,
                    piece,
                );
                let _ = conn.flush();
                if piece.is_empty() {
                    sends.swap_remove(i);
                } else {
                    send.sent = end;
                    send.last = Instant::now();
                }
            }
            Event::ClientMessage(m) if m.window == window && m.type_ == atoms.XWINDOW_QUIT => {
                return;
            }
            Event::SelectionNotify(n) if n.requestor == window => {
                let _ = events.send(event);
            }
            Event::PropertyNotify(p) if p.window == window => {
                let _ = events.send(event);
            }
            _ => {}
        }
    }
}

/// Writes the target `request` asks for to its property, then tells the
/// requestor whether it is there.
fn answer(
    conn: &RustConnection,
    atoms: &Atoms,
    offer: Option<&Offer>,
    request: &SelectionRequestEvent,
    sends: &mut Vec<Incremental>,
    chunk: usize,
) {
    let (requestor, target) = (request.requestor, request.target);
    // Clients older than ICCCM 2 name no property; the target stands in.
    let property = match request.property {
        NONE => target,
        property => property,
    };
    let served = offer
        .filter(|_| request.selection == atoms.CLIPBOARD)
        .and_then(|offer| {
            if target == atoms.TARGETS {
                let mut list = vec![atoms.TARGETS, atoms.TIMESTAMP];
                list.extend(offer.targets.iter().map(|t| t.0));
                conn.change_property32(
                    PropMode::REPLACE,
                    requestor,
                    property,
                    AtomEnum::ATOM,
                    &list,
                )
                .ok()
            } else if target == atoms.TIMESTAMP {
                conn.change_property32(
                    PropMode::REPLACE,
                    requestor,
                    property,
                    AtomEnum::INTEGER,
                    &[offer.time],
                )
                .ok()
            } else {
                let (_, type_, data) = offer.targets.iter().find(|t| t.0 == target)?;
                if data.len() <= chunk {
                    return conn
                        .change_property8(PropMode::REPLACE, requestor, property, *type_, data)
                        .ok();
                }
                // Too large for one request: announce its size, and send it a
                // piece each time the requestor deletes the property.
                let watch = ChangeWindowAttributesAux::new().event_mask(EventMask::PROPERTY_CHANGE);
                conn.change_window_attributes(requestor, &watch).ok()?;
                sends.retain(|s| (s.window, s.property) != (requestor, property));
                sends.push(Incremental {
                    window: requestor,
                    property,
                    type_: *type_,
                    data: data.clone(),
                    sent: 0,
                    last: Instant::now(),
                });
                let size = u32::try_from(data.len()).unwrap_or(u32::MAX);
                conn.change_property32(PropMode::REPLACE, requestor, property, atoms.INCR, &[size])
                    .ok()
            }
        });
    let notify = SelectionNotifyEvent {
        response_type: SELECTION_NOTIFY_EVENT,
        sequence: 0,
        time: request.time,
        requestor,
        selection: request.selection,
        target,
        property: if served.is_some() { property } else { NONE },
    };
    let _ = conn.send_event(false, requestor, EventMask::NO_EVENT, notify);
    let _ = conn.flush();
}
