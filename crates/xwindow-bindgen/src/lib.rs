//! xwindow's API declaration and browser IDL, and the x-idl generators an
//! adapter's build script runs over them. Every runtime gets the same
//! `window` classes, enums and method names from one declaration; only the
//! carriers differ.

use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};

pub use x_idl::haxe::{self};
pub use x_idl::wire;

/// The API every adapter exposes, before the appended key and cursor lists.
pub const WINDOW_API: &str = include_str!("../../../api/window.api.rs");
/// The browser APIs the page agent reaches, verbatim from W3C's webref.
pub const WINDOW_IDL: &str = include_str!("../../../api/spec/window.idl");
/// The page agent's own operations, and the event record it posts.
pub const AGENT_IDL: &str = include_str!("../../../api/spec/agent.idl");

/// The namespace the API is exposed under: the Haxe package and Caribou's
/// plugin name.
pub const NAMESPACE: &str = "window";
/// The native library: `xwindow.hdll` under HashLink, `xwindow_*` symbols
/// under Rayzor.
pub const LIBRARY: x_idl::Library<'static> = x_idl::Library("xwindow");
/// The dictionaries the page agent posts to the program's queue.
pub const POSTED: &[&str] = &["XwEvent"];

/// The runtime an adapter binds for.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Runtime {
    Caribou,
    HashLink,
    Rayzor,
}

/// The browser wire's schema: the DOM's APIs and the agent's.
pub fn browser_idl() -> String {
    format!("{WINDOW_IDL}\n{AGENT_IDL}")
}

/// The complete declaration: `WINDOW_API` with `KeyCode`, `NamedKey` and
/// `CursorIcon` from xwindow-core's lists, which the backends convert by
/// the same names. `Unrecognized` is a key a newer winit names.
pub fn window_api() -> String {
    let codes = xwindow_core::KEY_CODES.join(", ");
    let named = xwindow_core::NAMED_KEYS.join(", ");
    let icons = xwindow_core::CURSOR_ICONS.join(", ");
    format!(
        "{WINDOW_API}\n\
         /// A physical key, by its `KeyboardEvent.code` name.\n\
         enum KeyCode {{ Unrecognized, {codes} }}\n\
         /// A named logical key, by its `KeyboardEvent.key` name.\n\
         enum NamedKey {{ Unrecognized, {named} }}\n\
         enum CursorIcon {{ {icons} }}\n"
    )
}

/// Run `generate` over the declaration, written where x-idl reads it: a
/// file of this call's own, so parallel builds never share one.
fn with_declaration<T>(
    generate: impl FnOnce(Option<PathBuf>) -> Result<T, String>,
) -> Result<T, String> {
    static CALLS: AtomicUsize = AtomicUsize::new(0);
    let path = std::env::temp_dir().join(format!(
        "window.api.{}.{}.rs",
        std::process::id(),
        CALLS.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::write(&path, window_api()).map_err(|e| format!("window.api.rs: {e}"))?;
    let result = generate(Some(path.clone()));
    std::fs::remove_file(&path).ok();
    result
}

/// The typed model and ABI for `runtime`, to `include!` at the crate root
/// beside `mod backend`:
///
/// - Caribou: the model and its `plugin!` table.
/// - HashLink: the model and its `DEFINE_PRIM` resolvers, in `xwindow`.
/// - Rayzor: the model, `XIDL_METHODS` and `xidl_runtime_symbols()`.
pub fn generate(runtime: Runtime) -> Result<String, String> {
    let idl = browser_idl();
    with_declaration(|declaration| match runtime {
        Runtime::Caribou => x_idl::generate_caribou(NAMESPACE, declaration, &idl),
        Runtime::HashLink => LIBRARY.generate_hashlink(NAMESPACE, declaration, &idl),
        Runtime::Rayzor => LIBRARY.generate_rayzor(NAMESPACE, declaration, &idl, &[]),
    })
}

/// The `backend` module for a browser build: each function `implemented`
/// (xwindow-backend's `WEB`) defines forwards to `crate::web`, and any other
/// raises that it is not available in a page.
pub fn web_backend(runtime: Runtime, implemented: &str) -> Result<String, String> {
    let idl = browser_idl();
    with_declaration(|declaration| match runtime {
        Runtime::Caribou => x_idl::web_backend(NAMESPACE, declaration, &idl, implemented),
        // Rayzor's model takes the same carriers by the same names.
        Runtime::HashLink | Runtime::Rayzor => {
            x_idl::hashlink_web_backend(NAMESPACE, declaration, &idl, implemented)
        }
    })
}

/// The browser wire: Rust for `crate::wire`, and the JavaScript a page
/// serves as `xwindow_wire.mjs` beside the agent.
pub fn browser_wire() -> Result<wire::Wire, String> {
    wire::wire_posting(&browser_idl(), POSTED)
}

/// The conventional Haxe surface for HashLink/Ash or Rayzor, in package
/// `window`.
pub fn haxe(runtime: haxe::Runtime) -> Result<Vec<haxe::File>, String> {
    let idl = browser_idl();
    with_declaration(|declaration| LIBRARY.haxe(NAMESPACE, declaration, &idl, runtime))
}
