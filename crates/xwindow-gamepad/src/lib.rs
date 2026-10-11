//! Desktop controller input, on its own native thread, waking the window loop on changes.
use std::sync::{Arc, Mutex, MutexGuard};
use xwindow_core::gamepad::{Device, Event, State};

#[cfg(any(
    target_os = "linux",
    target_os = "freebsd",
    target_os = "windows",
    target_os = "macos"
))]
mod desktop;

pub struct Hub {
    state: Arc<Mutex<State>>,
    error: String,
    #[cfg(any(
        target_os = "linux",
        target_os = "freebsd",
        target_os = "windows",
        target_os = "macos"
    ))]
    worker: Option<desktop::Worker>,
}
impl Hub {
    pub fn new(wake: fn()) -> Self {
        let state = Arc::new(Mutex::new(State::new()));
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows",
            target_os = "macos"
        ))]
        {
            match desktop::Worker::start(state.clone(), wake) {
                Ok(worker) => Self {
                    state,
                    error: String::new(),
                    worker: Some(worker),
                },
                Err(error) => Self {
                    state,
                    error,
                    worker: None,
                },
            }
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows",
            target_os = "macos"
        )))]
        {
            let _ = wake;
            Self {
                state,
                error: "Gamepad input is unavailable on this native platform".into(),
            }
        }
    }
    pub fn available(&self) -> bool {
        self.error.is_empty()
    }
    pub fn error(&self) -> &str {
        &self.error
    }
    fn state(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(|e| e.into_inner())
    }
    pub fn count(&self) -> i32 {
        self.state().devices.iter().count() as i32
    }
    pub fn at(&self, index: i32) -> i32 {
        if index < 0 {
            0
        } else {
            self.state()
                .devices
                .iter()
                .nth(index as usize)
                .map_or(0, |(id, _)| id)
        }
    }
    pub fn read<T>(&self, id: i32, miss: T, read: impl FnOnce(&Device) -> T) -> T {
        self.state().devices.get(id).map_or(miss, read)
    }
    pub fn drain(&self, output: &mut [u8]) -> usize {
        self.state().drain(output)
    }
    pub fn poll(&self) -> Option<Event> {
        self.state().poll()
    }
    pub fn rumble(&self, id: i32, strong: f64, weak: f64, seconds: f64) -> bool {
        if !xwindow_core::gamepad::rumble_valid(strong, weak, seconds)
            || !self.read(id, false, |d| d.rumble)
        {
            return false;
        }
        #[cfg(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows",
            target_os = "macos"
        ))]
        {
            self.worker
                .as_ref()
                .is_some_and(|w| w.rumble(id, strong, weak, seconds))
        }
        #[cfg(not(any(
            target_os = "linux",
            target_os = "freebsd",
            target_os = "windows",
            target_os = "macos"
        )))]
        {
            false
        }
    }
}
