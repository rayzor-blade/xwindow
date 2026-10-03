//! The general pasteboard. Its types are UTIs, which UniformTypeIdentifiers
//! maps to and from MIME types; text is the pasteboard's string, and each
//! file in a `text/uri-list` its own pasteboard item, as Finder writes them.

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_app_kit::{
    NSPasteboard, NSPasteboardItem, NSPasteboardTypeFileURL, NSPasteboardTypeString,
    NSPasteboardWriting,
};
use objc2_foundation::{NSArray, NSData, NSString};
use objc2_uniform_type_identifiers::UTType;
use raw_window_handle::RawDisplayHandle;

use crate::{TEXT, URI_LIST};

pub struct Clipboard;

impl Clipboard {
    pub unsafe fn connect(_: RawDisplayHandle) -> Result<Self, String> {
        Ok(Self)
    }

    pub fn types(&self) -> Vec<String> {
        let board = unsafe { NSPasteboard::generalPasteboard() };
        let mut types = Vec::new();
        for uti in unsafe { board.types() }.iter().flat_map(|t| t.iter()) {
            if let Some(mime) = mime_of(&uti.to_string())
                && !types.contains(&mime)
            {
                types.push(mime);
            }
        }
        types
    }

    pub fn read(&self, mime: &str) -> Option<Vec<u8>> {
        let board = unsafe { NSPasteboard::generalPasteboard() };
        match mime {
            TEXT => unsafe { board.stringForType(NSPasteboardTypeString) }
                .map(|text| text.to_string().into_bytes()),
            URI_LIST => {
                let items = unsafe { board.pasteboardItems() }?;
                let uris: String = items
                    .iter()
                    .filter_map(|item| unsafe { item.stringForType(NSPasteboardTypeFileURL) })
                    .map(|uri| format!("{uri}\r\n"))
                    .collect();
                (!uris.is_empty()).then(|| uris.into_bytes())
            }
            _ => {
                let uti = NSString::from_str(&uti_of(mime)?);
                unsafe { board.dataForType(&uti) }.map(|data| data.bytes().to_vec())
            }
        }
    }

    pub fn write(&mut self, items: &[(String, Vec<u8>)]) -> bool {
        let first = unsafe { NSPasteboardItem::new() };
        let mut more = Vec::new();
        for (mime, bytes) in items {
            let done = match mime.as_str() {
                TEXT => {
                    let text = NSString::from_str(&String::from_utf8_lossy(bytes));
                    unsafe { first.setString_forType(&text, NSPasteboardTypeString) }
                }
                URI_LIST => {
                    let uris = String::from_utf8_lossy(bytes);
                    let mut uris = uris
                        .lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty() && !line.starts_with('#'));
                    let set = |item: &NSPasteboardItem, uri: &str| unsafe {
                        item.setString_forType(&NSString::from_str(uri), NSPasteboardTypeFileURL)
                    };
                    uris.next().is_some_and(|uri| set(&first, uri))
                        && uris.all(|uri| {
                            let item = unsafe { NSPasteboardItem::new() };
                            let done = set(&item, uri);
                            more.push(item);
                            done
                        })
                }
                _ => match uti_of(mime) {
                    Some(uti) => unsafe {
                        first.setData_forType(&NSData::with_bytes(bytes), &NSString::from_str(&uti))
                    },
                    None => false,
                },
            };
            if !done {
                return false;
            }
        }
        let objects: Vec<Retained<ProtocolObject<dyn NSPasteboardWriting>>> =
            std::iter::once(first)
                .chain(more)
                .map(ProtocolObject::from_retained)
                .collect();
        let board = unsafe { NSPasteboard::generalPasteboard() };
        unsafe { board.clearContents() };
        unsafe { board.writeObjects(&NSArray::from_vec(objects)) }
    }
}

/// The MIME type of a pasteboard type, if it has one.
fn mime_of(uti: &str) -> Option<String> {
    match uti {
        "public.utf8-plain-text" | "public.plain-text" | "NSStringPboardType" => {
            Some(TEXT.to_owned())
        }
        "public.file-url" => Some(URI_LIST.to_owned()),
        _ => {
            let ty = unsafe { UTType::typeWithIdentifier(&NSString::from_str(uti)) }?;
            let mime = unsafe { ty.preferredMIMEType() }?.to_string();
            Some(crate::essence(&mime))
        }
    }
}

/// The pasteboard type of a MIME type: a dynamic UTI when the system knows
/// none, which maps back to the same MIME type.
fn uti_of(mime: &str) -> Option<String> {
    let ty = unsafe { UTType::typeWithMIMEType(&NSString::from_str(mime)) }?;
    Some(unsafe { ty.identifier() }.to_string())
}
