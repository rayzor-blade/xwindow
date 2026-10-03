//! X11 or Wayland, whichever the window's display is.

use raw_window_handle::RawDisplayHandle;

#[path = "x11.rs"]
mod x11;

#[path = "wayland.rs"]
mod wayland;

pub enum Clipboard {
    X11(x11::Clipboard),
    Wayland(wayland::Clipboard),
}

impl Clipboard {
    pub unsafe fn connect(display: RawDisplayHandle) -> Result<Self, String> {
        match display {
            RawDisplayHandle::Wayland(handle) => Ok(Self::Wayland(unsafe {
                wayland::Clipboard::connect(handle.display)
            }?)),
            RawDisplayHandle::Xlib(_) | RawDisplayHandle::Xcb(_) => {
                Ok(Self::X11(x11::Clipboard::connect()?))
            }
            _ => Err("clipboard: the display is neither X11 nor Wayland".to_owned()),
        }
    }

    pub fn types(&self) -> Vec<String> {
        match self {
            Self::X11(clipboard) => clipboard.types(),
            Self::Wayland(clipboard) => clipboard.types(),
        }
    }

    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        match self {
            Self::X11(clipboard) => clipboard.read(mime),
            Self::Wayland(clipboard) => clipboard.read(mime),
        }
    }

    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        match self {
            Self::X11(clipboard) => clipboard.write(items),
            Self::Wayland(clipboard) => clipboard.write(items),
        }
    }
}
