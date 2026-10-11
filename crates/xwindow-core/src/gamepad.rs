//! Shared controller snapshots, bounded events and connection-scoped handles.
use crate::{Kind, Slab};
use std::collections::{HashMap, VecDeque};

pub const BUTTONS: usize = 17;
pub const AXES: usize = 4;
pub const SNAPSHOT_BYTES: usize = 176;
pub const EVENT_BYTES: usize = 32;
const QUEUE_LIMIT: usize = 1024;

#[derive(Clone, Debug, PartialEq)]
pub enum Event {
    Connected(i32),
    Disconnected(i32),
    Button {
        id: i32,
        button: usize,
        value: f64,
        pressed: bool,
    },
    Axis {
        id: i32,
        axis: usize,
        value: f64,
    },
    Overflow(u64),
}

pub struct Device {
    pub source: usize,
    pub name: String,
    pub buttons: u32,
    pub axes: u8,
    pub rumble: bool,
    pub values: [f64; BUTTONS],
    pub sticks: [f64; AXES],
    pub pressed: u32,
    pub sequence: u32,
}

impl Device {
    pub fn snapshot(&self, output: &mut [u8]) -> bool {
        if output.len() < SNAPSHOT_BYTES {
            return false;
        }
        for (i, value) in self.values.iter().chain(self.sticks.iter()).enumerate() {
            output[i * 8..i * 8 + 8].copy_from_slice(&value.to_le_bytes());
        }
        output[168..172].copy_from_slice(&self.pressed.to_le_bytes());
        output[172..176].copy_from_slice(&self.sequence.to_le_bytes());
        true
    }
}

pub struct State {
    pub devices: Slab<Device>,
    sources: HashMap<usize, i32>,
    queue: VecDeque<Event>,
    dropped: u64,
}
impl Default for State {
    fn default() -> Self {
        Self::new()
    }
}
impl State {
    pub fn new() -> Self {
        Self {
            devices: Slab::new(Kind::Gamepad),
            sources: HashMap::new(),
            queue: VecDeque::with_capacity(QUEUE_LIMIT),
            dropped: 0,
        }
    }
    fn push(&mut self, event: Event) {
        if self.queue.len() == QUEUE_LIMIT {
            self.queue.pop_front();
            self.dropped = self.dropped.saturating_add(1);
        }
        self.queue.push_back(event);
    }
    pub fn connect(
        &mut self,
        source: usize,
        name: String,
        buttons: u32,
        axes: u8,
        rumble: bool,
    ) -> i32 {
        if let Some(id) = self.sources.get(&source) {
            return *id;
        }
        let id = self.devices.put(Device {
            source,
            name,
            buttons: buttons & ((1 << BUTTONS) - 1),
            axes: axes & 15,
            rumble,
            values: [0.0; BUTTONS],
            sticks: [0.0; AXES],
            pressed: 0,
            sequence: 0,
        });
        if id != 0 {
            self.sources.insert(source, id);
            self.push(Event::Connected(id));
        }
        id
    }
    pub fn handle(&self, source: usize) -> i32 {
        self.sources.get(&source).copied().unwrap_or(0)
    }
    pub fn disconnect(&mut self, source: usize) -> bool {
        let Some(id) = self.sources.remove(&source) else {
            return false;
        };
        self.devices.remove(id);
        self.push(Event::Disconnected(id));
        true
    }
    pub fn button(&mut self, source: usize, button: usize, value: f64, pressed: bool) -> bool {
        if button >= BUTTONS || !value.is_finite() {
            return false;
        }
        let id = self.handle(source);
        let Some(device) = self.devices.get_mut(id) else {
            return false;
        };
        if device.buttons & (1 << button) == 0 {
            return false;
        }
        let value = value.clamp(0.0, 1.0);
        if device.values[button] == value && (device.pressed & (1 << button) != 0) == pressed {
            return false;
        }
        device.values[button] = value;
        if pressed {
            device.pressed |= 1 << button;
        } else {
            device.pressed &= !(1 << button);
        }
        device.sequence = device.sequence.wrapping_add(1);
        self.push(Event::Button {
            id,
            button,
            value,
            pressed,
        });
        true
    }
    pub fn axis(&mut self, source: usize, axis: usize, value: f64) -> bool {
        if axis >= AXES || !value.is_finite() {
            return false;
        }
        let id = self.handle(source);
        let Some(device) = self.devices.get_mut(id) else {
            return false;
        };
        if device.axes & (1 << axis) == 0 {
            return false;
        }
        let value = value.clamp(-1.0, 1.0);
        if device.sticks[axis] == value {
            return false;
        }
        device.sticks[axis] = value;
        device.sequence = device.sequence.wrapping_add(1);
        self.push(Event::Axis { id, axis, value });
        true
    }
    pub fn drain(&mut self, output: &mut [u8]) -> usize {
        let mut count = 0;
        for record in output.chunks_exact_mut(EVENT_BYTES) {
            let Some(event) = self.poll() else { break };
            let (kind, id, control, pressed, value, dropped) = match event {
                Event::Connected(id) => (1u32, id, 0u32, 0u32, 0.0f64, 0u64),
                Event::Disconnected(id) => (2, id, 0, 0, 0.0, 0),
                Event::Button {
                    id,
                    button,
                    value,
                    pressed,
                } => (3, id, button as u32, u32::from(pressed), value, 0),
                Event::Axis { id, axis, value } => (4, id, axis as u32, 0, value, 0),
                Event::Overflow(dropped) => (5, 0, 0, 0, 0.0, dropped.min(i64::MAX as u64)),
            };
            record[0..4].copy_from_slice(&kind.to_le_bytes());
            record[4..8].copy_from_slice(&id.to_le_bytes());
            record[8..12].copy_from_slice(&control.to_le_bytes());
            record[12..16].copy_from_slice(&pressed.to_le_bytes());
            record[16..24].copy_from_slice(&value.to_le_bytes());
            record[24..32].copy_from_slice(&dropped.to_le_bytes());
            count += 1;
        }
        count
    }
    pub fn poll(&mut self) -> Option<Event> {
        if self.dropped != 0 {
            return Some(Event::Overflow(std::mem::take(&mut self.dropped)));
        }
        self.queue.pop_front()
    }
}

pub fn rumble_valid(strong: f64, weak: f64, seconds: f64) -> bool {
    strong.is_finite()
        && weak.is_finite()
        && seconds.is_finite()
        && (0.0..=1.0).contains(&strong)
        && (0.0..=1.0).contains(&weak)
        && (0.0..=5.0).contains(&seconds)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn events_and_atomic_bytes_match_known_controls() {
        let mut s = State::new();
        let id = s.connect(7, "pad".into(), (1 << 17) - 1, 15, true);
        assert!(s.button(7, 6, 0.25, false));
        assert!(s.button(7, 0, 1.0, true));
        assert!(s.axis(7, 1, -0.375));
        let mut out = [0; 176];
        assert!(s.devices.get(id).unwrap().snapshot(&mut out));
        assert_eq!(f64::from_le_bytes(out[48..56].try_into().unwrap()), 0.25);
        assert_eq!(
            f64::from_le_bytes(out[144..152].try_into().unwrap()),
            -0.375
        );
        assert_eq!(u32::from_le_bytes(out[168..172].try_into().unwrap()), 1);
        assert_eq!(u32::from_le_bytes(out[172..176].try_into().unwrap()), 3);
        assert_eq!(s.poll(), Some(Event::Connected(id)));
        assert_eq!(
            s.poll(),
            Some(Event::Button {
                id,
                button: 6,
                value: 0.25,
                pressed: false
            })
        );
        assert_eq!(
            s.poll(),
            Some(Event::Button {
                id,
                button: 0,
                value: 1.0,
                pressed: true
            })
        );
        assert_eq!(
            s.poll(),
            Some(Event::Axis {
                id,
                axis: 1,
                value: -0.375
            })
        );
        assert_eq!(s.poll(), None);
        assert!(!s.axis(7, 1, f64::NAN));
        assert!(!s.button(7, 17, 1.0, true));
        let mut short = [42; 175];
        assert!(!s.devices.get(id).unwrap().snapshot(&mut short));
        assert_eq!(short, [42; 175]);
    }
    #[test]
    fn reusable_event_records_preserve_press_release_edges() {
        let mut s = State::new();
        let id = s.connect(7, "pad".into(), 1, 1, false);
        s.button(7, 0, 1.0, true);
        s.button(7, 0, 0.0, false);
        s.axis(7, 0, -0.5);
        s.disconnect(7);
        let mut short = [99; 31];
        assert_eq!(s.drain(&mut short), 0);
        assert_eq!(short, [99; 31]);
        let mut out = [99; 32 * 6 + 1];
        assert_eq!(s.drain(&mut out), 5);
        let read_u32 = |at| u32::from_le_bytes(out[at..at + 4].try_into().unwrap());
        for (i, kind) in [1, 3, 3, 4, 2].into_iter().enumerate() {
            assert_eq!(read_u32(i * 32), kind);
            assert_eq!(read_u32(i * 32 + 4), id as u32);
        }
        assert_eq!(read_u32(32 + 12), 1);
        assert_eq!(read_u32(64 + 12), 0);
        assert_eq!(f64::from_le_bytes(out[48..56].try_into().unwrap()), 1.0);
        assert_eq!(f64::from_le_bytes(out[80..88].try_into().unwrap()), 0.0);
        assert_eq!(f64::from_le_bytes(out[112..120].try_into().unwrap()), -0.5);
        assert_eq!(&out[160..], &[99; 33]);
        assert_eq!(s.drain(&mut out), 0);
    }
    #[test]
    fn reconnects_never_resurrect_an_old_handle() {
        let mut s = State::new();
        let original = s.connect(2, "pad".into(), 1, 1, false);
        for _ in 0..260 {
            assert!(s.disconnect(2));
            let now = s.connect(2, "pad".into(), 1, 1, false);
            assert_ne!(original, now);
            assert!(s.devices.get(original).is_none());
        }
        let id = s.handle(2);
        assert!(!s.devices.get(id).unwrap().rumble);
        assert_eq!(s.devices.get(id).unwrap().pressed, 0);
    }
    #[test]
    fn overflow_is_bounded_and_snapshots_stay_current() {
        let mut s = State::new();
        let id = s.connect(0, "pad".into(), 1, 1, false);
        for i in 0..1200 {
            s.axis(0, 0, if i % 2 == 0 { 1.0 } else { -1.0 });
        }
        assert_eq!(s.queue.len(), 1024);
        assert_eq!(s.poll(), Some(Event::Overflow(177)));
        assert_eq!(s.devices.get(id).unwrap().sticks[0], -1.0);
        for (a, b, t) in [(f64::NAN, 0.0, 1.0), (1.1, 0.0, 1.0), (0.0, 0.0, 5.1)] {
            assert!(!rumble_valid(a, b, t));
        }
        assert!(rumble_valid(1.0, 0.5, 0.0));
    }
}
