//! Platforms without a clipboard here.

use raw_window_handle::RawDisplayHandle;

pub struct Clipboard;

impl Clipboard {
    pub unsafe fn connect(_: RawDisplayHandle) -> Result<Self, String> {
        Err("clipboard: not on this platform".to_owned())
    }

    pub fn types(&self) -> Vec<String> {
        Vec::new()
    }

    pub fn read(&self, _: &str) -> Option<Vec<u8>> {
        None
    }

    pub fn write(&mut self, _: &[(String, Vec<u8>)]) -> bool {
        false
    }
}
