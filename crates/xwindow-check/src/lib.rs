//! A stand-in adapter: xwindow's generated Rayzor-shaped model and its
//! native or web backend, over carriers that only have the shapes a
//! runtime's do. It checks the generated code and the templates compile
//! together without any runtime's crates; nothing here is for use.

#![allow(dead_code, non_snake_case, clippy::all)]
#![cfg_attr(
    all(target_os = "wasi", target_feature = "atomics"),
    feature(stdarch_wasm_atomic_wait)
)]

mod runtime;

#[cfg(not(target_os = "wasi"))]
mod backend {
    include!(concat!(env!("OUT_DIR"), "/xwindow_backend/native.rs"));
}
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
}
