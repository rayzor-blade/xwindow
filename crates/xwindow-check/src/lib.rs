//! A stand-in adapter: xwindow's generated Rayzor-shaped model and its
//! native or web backend, over carriers that only have the shapes a
//! runtime's do. It checks the generated code and the templates compile
//! together without any runtime's crates; nothing here is for use beyond
//! the desktop test, which drives `backend` directly.

#![allow(dead_code, non_snake_case, clippy::all)]
#![cfg_attr(
    all(target_os = "wasi", target_feature = "atomics"),
    feature(stdarch_wasm_atomic_wait)
)]

pub mod runtime;

#[cfg(not(target_os = "wasi"))]
pub mod backend {
    include!(concat!(env!("OUT_DIR"), "/xwindow_backend/native.rs"));
}
/// The host's hook, as an adapter re-exports it, with the winit its event
/// loop is built from.
#[cfg(not(target_os = "wasi"))]
pub use backend::{Drive, attach};
#[cfg(not(target_os = "wasi"))]
pub use winit;
#[cfg(target_os = "wasi")]
mod web {
    include!(concat!(env!("OUT_DIR"), "/xwindow_backend/web.rs"));
}
#[cfg(target_os = "wasi")]
mod backend {
    use super::*;
    include!(concat!(env!("OUT_DIR"), "/window_web_backend.rs"));
}
#[cfg(target_os = "wasi")]
#[allow(non_camel_case_types, unused_variables)]
mod wire {
    include!(concat!(env!("OUT_DIR"), "/window_wire.rs"));
}

/// The descriptor Rayzor's plugin crate defines, as the registration names it.
mod rayzor_plugin {
    #[repr(C)]
    pub struct NativeMethodDesc {
        pub symbol_name: *const u8,
        pub symbol_name_len: usize,
        pub class_name: *const u8,
        pub class_name_len: usize,
        pub method_name: *const u8,
        pub method_name_len: usize,
        pub is_static: u8,
        pub param_count: u8,
        pub return_type: u8,
        pub param_types: [u8; 16],
    }
    unsafe impl Sync for NativeMethodDesc {}
}

#[allow(unused_imports)]
use runtime::{Buffer, BufferMut, Enum, ErrorKind, Future, NativeEnum, Rooted, Text, host};
include!(concat!(env!("OUT_DIR"), "/window.rs"));

#[cfg(test)]
mod tests {
    #[test]
    fn every_method_has_a_symbol_and_a_descriptor() {
        let symbols = super::xidl_runtime_symbols();
        assert_eq!(symbols.len(), super::XIDL_METHODS.len());
        assert!(
            symbols
                .iter()
                .any(|(name, _)| *name == "xwindow_window_poll_variant")
        );
        assert!(symbols.iter().all(|(name, _)| name.starts_with("xwindow_")));
    }

    #[test]
    fn getters_read_the_kept_event_down_through_its_nested_values() {
        use super::*;
        __XIDL_Window_Event.with(|slot| {
            *slot.borrow_mut() = Event::KeyboardInput {
                device_id: 3,
                event: KeyEvent::Input {
                    physical_key: PhysicalKey::Code {
                        code: KeyCode::KeyA,
                    },
                    logical_key: Key::Character { text: "a".into() },
                    text: OptionalText::Some { text: "a".into() },
                    location: KeyLocation::Left,
                    state: MouseElementState::Pressed,
                    repeat: true,
                    supplement: KeySupplement::Unavailable,
                },
                is_synthetic: false,
            }
        });
        assert_eq!(Window::eventKeyboardInputDeviceId(), 3);
        assert_eq!(Window::eventKeyboardInputEventVariant(), 0);
        assert_eq!(Window::eventKeyboardInputEventInputPhysicalKeyVariant(), 0);
        assert_eq!(
            Window::eventKeyboardInputEventInputPhysicalKeyCodeCode().get(),
            KeyCode::KeyA
        );
        assert_eq!(Window::eventKeyboardInputEventInputLogicalKeyVariant(), 1);
        assert_eq!(
            Window::eventKeyboardInputEventInputLogicalKeyCharacterText().as_str(),
            "a"
        );
        assert_eq!(Window::eventKeyboardInputEventInputTextVariant(), 1);
        assert_eq!(
            Window::eventKeyboardInputEventInputLocation().get(),
            KeyLocation::Left
        );
        assert!(Window::eventKeyboardInputEventInputRepeat());
        assert_eq!(Window::eventKeyboardInputEventInputSupplementVariant(), 0);
        // A field of another shape reads as its default.
        assert_eq!(Window::eventResizedWidth(), 0);

        __XIDL_Window_Event.with(|slot| {
            *slot.borrow_mut() = Event::DroppedFile {
                path: FilePath::UnixBytes {
                    bytes: VariantBytes(vec![b'/', 255]),
                },
            }
        });
        assert_eq!(Window::eventDroppedFilePathVariant(), 1);
        let bytes = Window::eventDroppedFilePathUnixBytesBytes();
        assert_eq!(unsafe { bytes.as_slice() }, [b'/', 255]);
    }
}
