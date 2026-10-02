//! A window as a GPU library takes it: a platform code and four integers,
//! the fields of raw-window-handle's handles. Two libraries share no Rust
//! type, only these numbers. `GpuInstance.surface(platform, wa, wb, da, db)`
//! in xgpu reads them by the same table.
//!
//! | Code | Platform | `wa` | `wb` | `da` | `db` |
//! |---|---|---|---|---|---|
//! | 1 | AppKit | `NSView*` | | | |
//! | 2 | Win32 | `HWND` | `HINSTANCE` | | |
//! | 3 | Xlib | window | visual id | `Display*` | screen |
//! | 4 | Wayland | `wl_surface*` | | `wl_display*` | |
//! | 5 | Android NDK | `ANativeWindow*` | | | |
//! | 6 | UIKit | `UIView*` | `UIViewController*` | | |
//! | 7 | Web canvas | canvas object id | | | |
//! | 8 | Web OffscreenCanvas | canvas object id | | | |

use std::ffi::c_void;
use std::num::NonZeroIsize;
use std::ptr::NonNull;

use raw_window_handle::{self as rwh, RawDisplayHandle, RawWindowHandle};

pub const NONE: i32 = 0;
pub const APPKIT: i32 = 1;
pub const WIN32: i32 = 2;
pub const XLIB: i32 = 3;
pub const WAYLAND: i32 = 4;
pub const ANDROID: i32 = 5;
pub const UIKIT: i32 = 6;
pub const WEB_CANVAS: i32 = 7;
pub const WEB_OFFSCREEN_CANVAS: i32 = 8;

/// A window's handles as a platform code and `[wa, wb, da, db]`, or `NONE`
/// for a platform outside the table.
pub fn split(window: RawWindowHandle, display: RawDisplayHandle) -> (i32, [i64; 4]) {
    let pointer = |p: NonNull<c_void>| p.as_ptr() as usize as i64;
    let optional = |p: Option<NonNull<c_void>>| p.map_or(0, pointer);
    match window {
        RawWindowHandle::AppKit(h) => (APPKIT, [pointer(h.ns_view), 0, 0, 0]),
        RawWindowHandle::Win32(h) => (
            WIN32,
            [
                h.hwnd.get() as i64,
                h.hinstance.map_or(0, |v| v.get() as i64),
                0,
                0,
            ],
        ),
        RawWindowHandle::Xlib(h) => {
            let (display, screen) = match display {
                RawDisplayHandle::Xlib(d) => (optional(d.display), d.screen as i64),
                _ => (0, 0),
            };
            (XLIB, [h.window as i64, h.visual_id as i64, display, screen])
        }
        RawWindowHandle::Wayland(h) => {
            let display = match display {
                RawDisplayHandle::Wayland(d) => pointer(d.display),
                _ => 0,
            };
            (WAYLAND, [pointer(h.surface), 0, display, 0])
        }
        RawWindowHandle::AndroidNdk(h) => (ANDROID, [pointer(h.a_native_window), 0, 0, 0]),
        RawWindowHandle::UiKit(h) => (
            UIKIT,
            [pointer(h.ui_view), optional(h.ui_view_controller), 0, 0],
        ),
        RawWindowHandle::WebCanvas(h) => (WEB_CANVAS, [pointer(h.obj), 0, 0, 0]),
        RawWindowHandle::WebOffscreenCanvas(h) => (WEB_OFFSCREEN_CANVAS, [pointer(h.obj), 0, 0, 0]),
        _ => (NONE, [0; 4]),
    }
}

/// The handles `split` took apart, or `None` for a code outside the table
/// or a field that cannot be null and is.
///
/// # Safety
///
/// The integers must come from `split` of a window that is still open.
pub unsafe fn join(platform: i32, raw: [i64; 4]) -> Option<(RawDisplayHandle, RawWindowHandle)> {
    let [wa, wb, da, db] = raw;
    let pointer = |v: i64| NonNull::new(v as usize as *mut c_void);
    Some(match platform {
        APPKIT => (
            RawDisplayHandle::AppKit(rwh::AppKitDisplayHandle::new()),
            RawWindowHandle::AppKit(rwh::AppKitWindowHandle::new(pointer(wa)?)),
        ),
        WIN32 => {
            let mut window = rwh::Win32WindowHandle::new(NonZeroIsize::new(wa as isize)?);
            window.hinstance = NonZeroIsize::new(wb as isize);
            (
                RawDisplayHandle::Windows(rwh::WindowsDisplayHandle::new()),
                RawWindowHandle::Win32(window),
            )
        }
        XLIB => {
            let mut window = rwh::XlibWindowHandle::new(wa as std::os::raw::c_ulong);
            window.visual_id = wb as std::os::raw::c_ulong;
            (
                RawDisplayHandle::Xlib(rwh::XlibDisplayHandle::new(pointer(da), db as i32)),
                RawWindowHandle::Xlib(window),
            )
        }
        WAYLAND => (
            RawDisplayHandle::Wayland(rwh::WaylandDisplayHandle::new(pointer(da)?)),
            RawWindowHandle::Wayland(rwh::WaylandWindowHandle::new(pointer(wa)?)),
        ),
        ANDROID => (
            RawDisplayHandle::Android(rwh::AndroidDisplayHandle::new()),
            RawWindowHandle::AndroidNdk(rwh::AndroidNdkWindowHandle::new(pointer(wa)?)),
        ),
        UIKIT => {
            let mut window = rwh::UiKitWindowHandle::new(pointer(wa)?);
            window.ui_view_controller = pointer(wb);
            (
                RawDisplayHandle::UiKit(rwh::UiKitDisplayHandle::new()),
                RawWindowHandle::UiKit(window),
            )
        }
        WEB_CANVAS => (
            RawDisplayHandle::Web(rwh::WebDisplayHandle::new()),
            RawWindowHandle::WebCanvas(rwh::WebCanvasWindowHandle::new(pointer(wa)?)),
        ),
        WEB_OFFSCREEN_CANVAS => (
            RawDisplayHandle::Web(rwh::WebDisplayHandle::new()),
            RawWindowHandle::WebOffscreenCanvas(rwh::WebOffscreenCanvasWindowHandle::new(pointer(
                wa,
            )?)),
        ),
        _ => return None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handles_come_back_together() {
        let view = NonNull::new(0x1000 as *mut c_void).unwrap();
        let window = RawWindowHandle::AppKit(rwh::AppKitWindowHandle::new(view));
        let display = RawDisplayHandle::AppKit(rwh::AppKitDisplayHandle::new());
        let (platform, raw) = split(window, display);
        assert_eq!((platform, raw), (APPKIT, [0x1000, 0, 0, 0]));
        assert_eq!(unsafe { join(platform, raw) }, Some((display, window)));

        let mut x = rwh::XlibWindowHandle::new(42);
        x.visual_id = 7;
        let x_display = rwh::XlibDisplayHandle::new(NonNull::new(0x2000 as *mut c_void), 1);
        let (platform, raw) = split(RawWindowHandle::Xlib(x), RawDisplayHandle::Xlib(x_display));
        assert_eq!((platform, raw), (XLIB, [42, 7, 0x2000, 1]));
        assert_eq!(
            unsafe { join(platform, raw) },
            Some((RawDisplayHandle::Xlib(x_display), RawWindowHandle::Xlib(x)))
        );
        assert_eq!(unsafe { join(APPKIT, [0; 4]) }, None);
        assert_eq!(unsafe { join(99, [1; 4]) }, None);
    }
}
