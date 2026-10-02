//! A program driven in turns: it opens a window in its first turn, reads
//! what has come in each, and checks what it saw. Shared with the mobile
//! harnesses, which give it turns from a host's loop.

use std::time::{Duration, Instant};

use xwindow_check::{Window, WindowAttributes};

/// `Event`'s variants, in the declaration's order.
const NAMES: [&str; 33] = [
    "None",
    "Closed",
    "Destroyed",
    "Resized",
    "Moved",
    "Focused",
    "Occluded",
    "ScaleFactorChanged",
    "ThemeChanged",
    "RedrawRequested",
    "CursorEntered",
    "CursorLeft",
    "CursorMoved",
    "MouseInput",
    "MouseWheel",
    "KeyboardInput",
    "ModifiersChanged",
    "Ime",
    "DroppedFile",
    "HoveredFile",
    "HoveredFileCancelled",
    "PinchGesture",
    "PanGesture",
    "DoubleTapGesture",
    "RotationGesture",
    "TouchpadPressure",
    "AxisMotion",
    "Touch",
    "ActivationTokenDone",
    "Device",
    "Resumed",
    "Suspended",
    "MemoryWarning",
];

const TURNS: u32 = 20;
/// What each turn asks: the next turn within this, when nothing comes.
const PACE: f64 = 0.05;

pub struct Program {
    window: Option<Box<Window>>,
    turns: u32,
    redraws: u32,
    seen: Vec<&'static str>,
    started: Option<Instant>,
    failures: Vec<String>,
}

impl Program {
    pub fn new() -> Self {
        Program {
            window: None,
            turns: 0,
            redraws: 0,
            seen: Vec::new(),
            started: None,
            failures: Vec::new(),
        }
    }

    /// One turn; false once the program is done.
    pub fn turn(&mut self, log: &dyn Fn(&str)) -> bool {
        self.turns += 1;
        let window = match &mut self.window {
            Some(window) => window,
            None => {
                let w = Window::open(&WindowAttributes::new());
                log(&format!(
                    "opened: valid {} {}x{} scale {} platform {} raw0 {:#x}",
                    Window::valid(&w),
                    Window::width(&w),
                    Window::height(&w),
                    Window::scaleFactor(&w),
                    Window::platform(&w),
                    Window::raw(&w, 0)
                ));
                if !Window::valid(&w) || Window::platform(&w) == 0 || Window::raw(&w, 0) == 0 {
                    self.failures.push("the window has no surface".to_owned());
                }
                if Window::width(&w) <= 0 || Window::height(&w) <= 0 {
                    self.failures.push("the window has no size".to_owned());
                }
                Window::requestRedraw(&w);
                self.started = Some(Instant::now());
                self.window.insert(w)
            }
        };
        loop {
            let name = NAMES
                .get(Window::pollVariant(window) as usize)
                .copied()
                .unwrap_or("?");
            if name == "None" {
                break;
            }
            if name == "RedrawRequested" {
                self.redraws += 1;
                if self.redraws < 3 {
                    Window::requestRedraw(window);
                }
            }
            if !self.seen.contains(&name) {
                log(&format!("first {name}"));
                self.seen.push(name);
            }
        }
        let _ = Window::waitVariant(window, PACE);
        if self.turns < TURNS {
            return true;
        }
        let took = self.started.map_or(Duration::ZERO, |s| s.elapsed());
        log(&format!(
            "{} turns in {} ms, {} redraws, saw {:?}",
            self.turns,
            took.as_millis(),
            self.redraws,
            self.seen
        ));
        if self.redraws < 3 {
            self.failures.push(format!("{} redraws of 3", self.redraws));
        }
        // Turns paced by `wait` cannot all come at once.
        if took < Duration::from_secs_f64(PACE * f64::from(TURNS) / 4.0) {
            self.failures
                .push(format!("{} turns took {} ms", self.turns, took.as_millis()));
        }
        Window::close(window);
        false
    }

    pub fn failures(&self) -> &[String] {
        &self.failures
    }
}
