//! Windows: clipboard formats mapped to MIME types. Text is CF_UNICODETEXT,
//! HTML the registered "HTML Format", PNG "PNG", RTF "Rich Text Format",
//! files CF_HDROP; any other type is a format registered under its own name.

use crate::windows_data as data;
use clipboard_win::formats::{CF_DIB, CF_DIBV5, CF_HDROP, CF_OEMTEXT, CF_TEXT, CF_UNICODETEXT};
use clipboard_win::options::NoClear;
use clipboard_win::raw;
use std::time::Duration;

/// The formats registered by name, which have no fixed number.
pub struct Clipboard {
    html: u32,
    png: u32,
    rtf: u32,
}

/// Registered formats are numbered from here; below are the predefined ones.
const REGISTERED: u32 = 0xc000;

impl Clipboard {
    pub unsafe fn connect(_: raw_window_handle::RawDisplayHandle) -> Result<Self, String> {
        let register =
            |name| register(name).ok_or_else(|| format!("clipboard: cannot register {name:?}"));
        Ok(Self {
            html: register("HTML Format")?,
            png: register("PNG")?,
            rtf: register("Rich Text Format")?,
        })
    }

    pub fn types(&self) -> Vec<String> {
        let Some(_open) = open() else {
            return Vec::new();
        };
        let mut types = Vec::new();
        for format in raw::EnumFormats::new() {
            if let Some(mime) = self.mime(format)
                && !types.contains(&mime)
            {
                types.push(mime);
            }
        }
        types
    }

    /// The MIME type a format on the open clipboard reads as, if any.
    fn mime(&self, format: u32) -> Option<String> {
        let mime = match format {
            CF_UNICODETEXT | CF_TEXT | CF_OEMTEXT => crate::TEXT,
            CF_HDROP => crate::URI_LIST,
            // A DIB reads as PNG when its header is one this converts.
            CF_DIB | CF_DIBV5 => {
                let mut header = [0; 124];
                let len = raw::get(format, &mut header).ok()?;
                data::dib_supported(&header[..len]).then_some("image/png")?
            }
            _ if format == self.html => "text/html",
            _ if format == self.png => "image/png",
            _ if format == self.rtf => "text/rtf",
            REGISTERED.. => return raw::format_name_big(format).filter(|name| name.contains('/')),
            _ => return None,
        };
        Some(mime.to_owned())
    }

    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        let _open = open()?;
        self.read_mapped(mime).or_else(|| bytes(register(mime)?))
    }

    /// `mime` from the format it maps to, on the open clipboard.
    fn read_mapped(&self, mime: &str) -> Option<Vec<u8>> {
        match mime {
            crate::TEXT => {
                // Windows converts CF_TEXT to this when only that was set.
                let mut text = Vec::new();
                raw::is_format_avail(CF_UNICODETEXT).then(|| raw::get_string(&mut text).ok())??;
                Some(text)
            }
            "text/html" => data::html_from_cf(&bytes(self.html)?),
            "image/png" => bytes(self.png)
                .map(|mut png| {
                    png.truncate(data::png_len(&png));
                    png
                })
                .or_else(|| data::dib_to_png(&bytes(CF_DIBV5)?))
                .or_else(|| data::dib_to_png(&bytes(CF_DIB)?)),
            "text/rtf" => {
                let mut rtf = bytes(self.rtf)?;
                // RTF writers end it with a NUL, as a C string.
                rtf.truncate(rtf.iter().position(|&b| b == 0).unwrap_or(rtf.len()));
                Some(rtf)
            }
            crate::URI_LIST => {
                let mut paths = Vec::new();
                raw::is_format_avail(CF_HDROP).then(|| raw::get_file_list(&mut paths).ok())??;
                Some(data::uri_list(&paths))
            }
            _ => None,
        }
    }

    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        let Some(_open) = open() else {
            return false;
        };
        if raw::empty().is_err() {
            return false;
        }
        // Every item is set, even after one fails, so the rest still reach
        // the clipboard.
        let mut ok = true;
        for (mime, bytes) in items {
            ok &= self.set(mime, bytes);
        }
        ok
    }

    /// Puts `mime` on the open, emptied clipboard.
    fn set(&self, mime: &str, bytes: &[u8]) -> bool {
        let set = |format, bytes: &[u8]| raw::set_without_clear(format, bytes).is_ok();
        match mime {
            // Windows offers it as CF_TEXT and CF_OEMTEXT too.
            crate::TEXT => raw::set_string_with(&String::from_utf8_lossy(bytes), NoClear).is_ok(),
            "text/html" => set(self.html, &data::cf_html(bytes)),
            "image/png" => set(self.png, bytes),
            "text/rtf" => set(self.rtf, bytes),
            crate::URI_LIST => {
                let paths = data::paths(bytes);
                !paths.is_empty() && raw::set_file_list_with(&paths, NoClear).is_ok()
            }
            _ => register(mime).is_some_and(|format| set(format, bytes)),
        }
    }
}

/// The clipboard, open until the guard drops. Another program may hold it for
/// a moment, so it is tried a few times, a little apart.
fn open() -> Option<clipboard_win::Clipboard> {
    for _ in 0..10 {
        if let Ok(open) = clipboard_win::Clipboard::new() {
            return Some(open);
        }
        std::thread::sleep(Duration::from_millis(5));
    }
    None
}

/// The format registered under `name`, registering it if no program has.
fn register(name: &str) -> Option<u32> {
    raw::register_format(name).map(|format| format.get())
}

/// A format's bytes on the open clipboard, if it holds that format.
fn bytes(format: u32) -> Option<Vec<u8>> {
    let mut out = Vec::new();
    raw::is_format_avail(format).then(|| raw::get_vec(format, &mut out).ok())??;
    Some(out)
}
