//! Installs xwindow's backend into a runtime adapter's build.
//!
//! The installed source expects the generated window model at the crate
//! root (or beneath a scope), its carriers under `crate::runtime`, the
//! `xwindow-core` crate, and, for the browser, the generated wire at
//! `crate::wire`. The native backend needs `winit` 0.30,
//! `raw-window-handle` 0.6 and `window_clipboard` 0.5. Runtime ABI code
//! stays in the adapter while every adapter compiles the same window
//! operations.

use std::io;
use std::path::{Path, PathBuf};

/// The winit backend, for desktops.
pub const NATIVE: &str = include_str!("template/native.rs");
/// The browser backend, which `xwindow_bindgen::web_backend` takes as the
/// functions a page implements.
pub const WEB: &str = include_str!("template/web.rs");
/// The page agent a host starts for a program in a page, beside the
/// generated wire as `xwindow_wire.mjs`.
pub const AGENT: &str = include_str!("template/agent.mjs");

/// What a host imports for the agent, and the wire module it imports.
pub const AGENT_MODULE: &str = "xwindow.mjs";
pub const WIRE_MODULE: &str = "xwindow_wire.mjs";

/// Source to `include!`: inner doc comments are not allowed there.
fn includable(source: &str, scope: Option<&str>) -> String {
    let source = source
        .lines()
        .map(|line| {
            line.strip_prefix("//!")
                .map_or(line.to_owned(), |doc| format!("//{doc}"))
        })
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    match scope {
        Some(scope) => source.replace("crate::", &format!("{scope}::")),
        None => source,
    }
}

/// Write `xwindow_backend/{native,web}.rs` and `page/xwindow.mjs` beneath
/// `out`, returning the native backend's path.
pub fn install(out: impl AsRef<Path>) -> io::Result<PathBuf> {
    install_with_scope(out.as_ref(), None)
}

/// Install the backend for a model nested beneath `scope`, such as
/// `crate::window`, so it does not collide with an adapter's own types.
pub fn install_scoped(out: impl AsRef<Path>, scope: &str) -> io::Result<PathBuf> {
    install_with_scope(out.as_ref(), Some(scope))
}

fn install_with_scope(out: &Path, scope: Option<&str>) -> io::Result<PathBuf> {
    let root = out.join("xwindow_backend");
    std::fs::create_dir_all(&root)?;
    let native = root.join("native.rs");
    std::fs::write(&native, includable(NATIVE, scope))?;
    std::fs::write(root.join("web.rs"), includable(WEB, scope))?;
    let page = out.join("page");
    std::fs::create_dir_all(&page)?;
    std::fs::write(page.join(AGENT_MODULE), AGENT)?;
    Ok(native)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn installed_backend_is_includable_and_runtime_neutral() {
        let out = std::env::temp_dir().join(format!("xwindow-backend-{}", std::process::id()));
        let native = install(&out).unwrap();
        let source = std::fs::read_to_string(native).unwrap();
        assert!(!source.contains("//!"));
        assert!(!source.contains("caribou_abi"));
        assert!(source.contains("crate::runtime"));
        assert!(out.join("page/xwindow.mjs").exists());
        std::fs::remove_dir_all(&out).unwrap();

        let scoped = install_scoped(&out, "crate::window").unwrap();
        let source = std::fs::read_to_string(scoped).unwrap();
        assert!(source.contains("crate::window::runtime"));
        std::fs::remove_dir_all(&out).unwrap();
    }
}
