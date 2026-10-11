use super::*;
use gilrs::ff::{BaseEffect, BaseEffectType, Effect, EffectBuilder, Repeat, Replay, Ticks};
use gilrs::{Axis, Button, EventType, Filter, GamepadId, Gilrs, GilrsBuilder};
use std::collections::HashMap;
use std::sync::mpsc::{self, SyncSender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

// W3C standard positions: shoulders and analog triggers have distinct controls.
const BUTTONS: [Button; 17] = [
    Button::South,
    Button::East,
    Button::West,
    Button::North,
    Button::LeftTrigger,
    Button::RightTrigger,
    Button::LeftTrigger2,
    Button::RightTrigger2,
    Button::Select,
    Button::Start,
    Button::LeftThumb,
    Button::RightThumb,
    Button::DPadUp,
    Button::DPadDown,
    Button::DPadLeft,
    Button::DPadRight,
    Button::Mode,
];
const AXES: [Axis; 4] = [
    Axis::LeftStickX,
    Axis::LeftStickY,
    Axis::RightStickX,
    Axis::RightStickY,
];

struct Rumble {
    id: i32,
    strong: f64,
    weak: f64,
    seconds: f64,
    reply: SyncSender<bool>,
}
enum Command {
    Rumble(Rumble),
    Stop,
}
pub(super) struct Worker {
    commands: SyncSender<Command>,
    thread: Option<JoinHandle<()>>,
}
impl Worker {
    pub fn start(state: Arc<Mutex<State>>, wake: fn()) -> Result<Self, String> {
        let (commands, receiver) = mpsc::sync_channel(32);
        let (ready, started) = mpsc::sync_channel(1);
        let thread = std::thread::Builder::new()
            .name("xwindow-gamepads".into())
            .spawn(move || {
                let mut gilrs = match GilrsBuilder::new()
                    .with_default_filters(false)
                    .set_update_state(false)
                    .build()
                {
                    Ok(g) => g,
                    Err(e) => {
                        let _ = ready.send(Err(e.to_string()));
                        return;
                    }
                };
                {
                    let mut s = state.lock().unwrap();
                    for (_, pad) in gilrs.gamepads() {
                        connect(&mut s, pad);
                    }
                }
                let _ = ready.send(Ok(()));
                wake();
                let mut effects: HashMap<GamepadId, (Effect, Instant)> = HashMap::new();
                loop {
                    while let Ok(command) = receiver.try_recv() {
                        match command {
                            Command::Stop => return,
                            Command::Rumble(r) => {
                                let source =
                                    state.lock().unwrap().devices.get(r.id).map(|d| d.source);
                                let id = source.and_then(|source| {
                                    gilrs
                                        .gamepads()
                                        .find(|(id, _)| usize::from(*id) == source)
                                        .map(|(id, _)| id)
                                });
                                let ok = id.is_some_and(|id| {
                                    rumble(
                                        &mut gilrs,
                                        &mut effects,
                                        id,
                                        r.strong,
                                        r.weak,
                                        r.seconds,
                                    )
                                });
                                let _ = r.reply.send(ok);
                            }
                        }
                    }
                    effects.retain(|_, (effect, end)| {
                        if Instant::now() >= *end {
                            let _ = effect.stop();
                            false
                        } else {
                            true
                        }
                    });
                    // Only this native thread waits periodically, to service rumble commands.
                    let event = gilrs
                        .next_event_blocking(Some(Duration::from_millis(16)))
                        .filter_ev(&gilrs::ev::filter::axis_dpad_to_button, &mut gilrs);
                    let Some(event) = event else { continue };
                    gilrs.update(&event);
                    let source = usize::from(event.id);
                    let changed = {
                        let mut s = state.lock().unwrap();
                        match event.event {
                            EventType::Connected => {
                                connect(&mut s, gilrs.gamepad(event.id));
                                true
                            }
                            EventType::Disconnected => {
                                effects.remove(&event.id);
                                s.disconnect(source)
                            }
                            EventType::ButtonPressed(button, code)
                            | EventType::ButtonReleased(button, code)
                            | EventType::ButtonChanged(button, _, code) => BUTTONS
                                .iter()
                                .position(|b| *b == button)
                                .is_some_and(|index| {
                                    let pad = gilrs.gamepad(event.id);
                                    let data = pad.state().button_data(code);
                                    let pressed = data.is_some_and(|d| d.is_pressed());
                                    let value = data.map_or(if pressed { 1.0 } else { 0.0 }, |d| {
                                        f64::from(d.value())
                                    });
                                    s.button(source, index, value, pressed)
                                }),
                            EventType::AxisChanged(axis, value, _) => AXES
                                .iter()
                                .position(|a| *a == axis)
                                .is_some_and(|i| s.axis(source, i, axis_value(i, value))),
                            _ => false,
                        }
                    };
                    if changed {
                        wake();
                    }
                }
            })
            .map_err(|e| e.to_string())?;
        match started.recv().map_err(|e| e.to_string())? {
            Ok(()) => Ok(Self {
                commands,
                thread: Some(thread),
            }),
            Err(e) => {
                let _ = thread.join();
                Err(e)
            }
        }
    }
    pub fn rumble(&self, id: i32, strong: f64, weak: f64, seconds: f64) -> bool {
        let (reply, response) = mpsc::sync_channel(1);
        self.commands
            .try_send(Command::Rumble(Rumble {
                id,
                strong,
                weak,
                seconds,
                reply,
            }))
            .is_ok()
            && response.recv().unwrap_or(false)
    }
}
impl Drop for Worker {
    fn drop(&mut self) {
        let _ = self.commands.send(Command::Stop);
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn axis_value(index: usize, value: f32) -> f64 {
    if index == 1 || index == 3 {
        -f64::from(value)
    } else {
        f64::from(value)
    }
}
fn connect(s: &mut State, pad: gilrs::Gamepad<'_>) {
    let mut buttons = BUTTONS.iter().enumerate().fold(0, |mask, (i, b)| {
        mask | if pad.button_code(*b).is_some() {
            1 << i
        } else {
            0
        }
    });
    if pad.axis_code(Axis::DPadX).is_some() {
        buttons |= (1 << 14) | (1 << 15);
    }
    if pad.axis_code(Axis::DPadY).is_some() {
        buttons |= (1 << 12) | (1 << 13);
    }
    let axes = AXES.iter().enumerate().fold(0, |mask, (i, a)| {
        mask | if pad.axis_code(*a).is_some() {
            1 << i
        } else {
            0
        }
    });
    let source = usize::from(pad.id());
    s.connect(
        source,
        pad.name().to_owned(),
        buttons,
        axes,
        pad.is_ff_supported(),
    );
    for (i, button) in BUTTONS.iter().enumerate() {
        if let Some(data) = pad.button_data(*button) {
            s.button(source, i, f64::from(data.value()), data.is_pressed());
        }
    }
    for (i, axis) in AXES.iter().enumerate() {
        if let Some(data) = pad.axis_data(*axis) {
            s.axis(source, i, axis_value(i, data.value()));
        }
    }
}

fn rumble(
    gilrs: &mut Gilrs,
    effects: &mut HashMap<GamepadId, (Effect, Instant)>,
    id: GamepadId,
    strong: f64,
    weak: f64,
    seconds: f64,
) -> bool {
    if let Some((old, _)) = effects.remove(&id) {
        let _ = old.stop();
    }
    if seconds == 0.0 {
        return true;
    }
    let duration = Duration::from_secs_f64(seconds);
    let ticks = Ticks::from_ms(duration.as_millis().max(1) as u32);
    let mut builder = EffectBuilder::new();
    for kind in [
        BaseEffectType::Strong {
            magnitude: (strong * 65535.0).round() as u16,
        },
        BaseEffectType::Weak {
            magnitude: (weak * 65535.0).round() as u16,
        },
    ] {
        builder.add_effect(BaseEffect {
            kind,
            scheduling: Replay {
                play_for: ticks,
                ..Default::default()
            },
            ..Default::default()
        });
    }
    match builder
        .gamepads(&[id])
        .repeat(Repeat::For(ticks))
        .finish(gilrs)
    {
        Ok(effect) => {
            if effect.play().is_err() {
                return false;
            }
            effects.insert(id, (effect, Instant::now() + duration));
            true
        }
        Err(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn canonical_indices_and_signs_are_independent_of_gilrs_ordinals() {
        assert_eq!(BUTTONS[2], Button::West);
        assert_eq!(BUTTONS[3], Button::North);
        assert_eq!(BUTTONS[4], Button::LeftTrigger);
        assert_eq!(BUTTONS[6], Button::LeftTrigger2);
        assert_eq!(BUTTONS[12], Button::DPadUp);
        assert_eq!(BUTTONS[16], Button::Mode);
        assert_eq!(axis_value(0, 0.125), 0.125);
        assert_eq!(axis_value(1, 0.375), -0.375);
        assert_eq!(axis_value(3, -1.0), 1.0);
    }
    #[test]
    fn backend_starts_without_a_window_and_shuts_down() {
        let hub = Hub::new(|| {});
        assert!(hub.available(), "{}", hub.error());
        assert_eq!(hub.at(-1), 0);
        assert!(!hub.rumble(0, 1.0, 1.0, 1.0));
    }
}
