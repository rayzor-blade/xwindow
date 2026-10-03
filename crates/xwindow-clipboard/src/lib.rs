//! The system clipboard's typed data: which types it holds, the bytes for
//! one, and several representations written at once, so that a reader takes
//! the richest it understands.
//!
//! Types are MIME types. `text/plain` is UTF-8 on every platform, whatever
//! the platform calls text; `text/uri-list` carries file lists. Other types
//! pass through as the platform names them: UTIs on macOS, clipboard formats
//! on Windows, targets on X11, MIME types on Wayland.

use raw_window_handle::RawDisplayHandle;

/// UTF-8 text.
pub const TEXT: &str = "text/plain";
/// Files, as `file://` URIs, one per line, each ended by CRLF.
pub const URI_LIST: &str = "text/uri-list";

#[cfg(target_os = "macos")]
#[path = "macos.rs"]
mod platform;

#[cfg(windows)]
#[path = "windows.rs"]
mod platform;

// The Windows formats' byte conversions; platform-neutral, so their tests run
// anywhere.
#[cfg(any(windows, test))]
mod windows_data;

#[cfg(all(
    unix,
    not(any(target_os = "macos", target_os = "ios", target_os = "android"))
))]
#[path = "linux.rs"]
mod platform;

#[cfg(not(any(
    target_os = "macos",
    windows,
    all(
        unix,
        not(any(target_os = "macos", target_os = "ios", target_os = "android"))
    )
)))]
#[path = "unsupported.rs"]
mod platform;

pub struct Clipboard {
    inner: platform::Clipboard,
}

impl Clipboard {
    /// The clipboard, through a window's display.
    ///
    /// # Safety
    ///
    /// `display` stays valid for as long as the clipboard lives.
    pub unsafe fn connect(display: RawDisplayHandle) -> Result<Self, String> {
        Ok(Self {
            inner: unsafe { platform::Clipboard::connect(display) }?,
        })
    }

    /// The types the clipboard holds now, best first, each once.
    pub fn types(&self) -> Vec<String> {
        self.inner.types()
    }

    /// The bytes for `mime`, or none when the clipboard does not hold it.
    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        self.inner.read(&essence(mime))
    }

    /// Replaces the clipboard's contents with `items`, each a type and its
    /// bytes; whether the platform took them.
    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        let items: Vec<(String, Vec<u8>)> = items
            .iter()
            .map(|(mime, bytes)| (essence(mime), bytes.clone()))
            .collect();
        !items.is_empty() && self.inner.write(&items)
    }
}

/// A MIME type without its parameters, lowercased: `text/plain;charset=utf-8`
/// is `text/plain`.
pub fn essence(mime: &str) -> String {
    mime.split(';')
        .next()
        .unwrap_or("")
        .trim()
        .to_ascii_lowercase()
}
