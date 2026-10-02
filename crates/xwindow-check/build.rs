use std::path::PathBuf;
use xwindow_bindgen::Runtime;

fn main() {
    let out = PathBuf::from(std::env::var_os("OUT_DIR").unwrap());
    xwindow_backend::install(&out).unwrap();
    let model = xwindow_bindgen::generate(Runtime::Rayzor).unwrap();
    std::fs::write(out.join("window.rs"), model).unwrap();
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("wasi") {
        let wire = xwindow_bindgen::browser_wire().unwrap();
        std::fs::write(out.join("window_wire.rs"), wire.rust).unwrap();
        let backend = xwindow_bindgen::web_backend(Runtime::Rayzor, xwindow_backend::WEB).unwrap();
        std::fs::write(out.join("window_web_backend.rs"), backend).unwrap();
    }
}
