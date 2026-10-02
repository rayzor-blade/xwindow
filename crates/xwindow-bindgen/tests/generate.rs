//! Every runtime's bindings generate from the one declaration, name the
//! same classes and methods, and reach every backend function.

use xwindow_bindgen::{Runtime, haxe};

#[test]
fn every_runtime_generates_its_model() {
    let caribou = xwindow_bindgen::generate(Runtime::Caribou).unwrap();
    syn::parse_file(&caribou).unwrap();
    assert!(caribou.contains("caribou_abi :: plugin ! { name : \"window\""));
    assert!(caribou.contains("# [caribou (name = \"window.Event\")]"));
    assert!(caribou.contains("fn poll (& Window) -> Enum < Event > ;"));

    let hashlink = xwindow_bindgen::generate(Runtime::HashLink).unwrap();
    syn::parse_file(&hashlink).unwrap();
    assert!(hashlink.contains("hlp_window_poll_variant"));
    assert!(hashlink.contains("\"PXxwindow_WindowAttributes__i\""));

    let rayzor = xwindow_bindgen::generate(Runtime::Rayzor).unwrap();
    syn::parse_file(&rayzor).unwrap();
    assert!(rayzor.contains("export_name = \"xwindow_window_poll_keyboard_input_code\""));
    assert!(rayzor.contains("\"window::Window\""));
}

#[test]
fn the_key_and_cursor_enums_come_from_the_core_lists() {
    let api = xwindow_bindgen::window_api();
    assert!(api.contains("enum KeyCode { Unidentified, Backquote,"));
    assert!(api.contains("enum Key { Unidentified, Character, Dead,"));
    assert!(api.contains("ZoomOut, DndAsk, AllResize }"));
    assert_eq!(xwindow_core::KEY_CODES.len(), 194);
    assert_eq!(xwindow_core::NAMED_KEYS.len(), 306);
}

#[test]
fn a_page_implements_every_backend_function() {
    for runtime in [Runtime::Caribou, Runtime::HashLink] {
        let backend = xwindow_bindgen::web_backend(runtime, xwindow_backend::WEB).unwrap();
        syn::parse_file(&backend).unwrap();
        assert!(
            !backend.contains("is not available on the web"),
            "a page leaves a function unimplemented"
        );
    }
}

#[test]
fn the_wire_posts_events_and_carries_the_dom() {
    let wire = xwindow_bindgen::browser_wire().unwrap();
    assert!(
        wire.js
            .contains("export function post_XwEvent(queue, value)")
    );
    assert!(wire.rust.contains("pub fn document_set_title"));
    assert!(
        wire.rust
            .contains("pub fn css_style_declaration_set_property")
    );
    assert!(wire.rust.contains("pub fn xw_agent_set_cursor_image"));
    assert!(wire.rust.contains("pub struct XwEvent"));
}

#[test]
fn both_haxe_surfaces_have_the_same_classes() {
    let names = |runtime| {
        let mut names: Vec<_> = xwindow_bindgen::haxe(runtime)
            .unwrap()
            .into_iter()
            .map(|f| f.path)
            .filter(|p| !p.ends_with("XidlBytes.hx"))
            .collect();
        names.sort();
        names
    };
    let hashlink = names(haxe::Runtime::HashLink);
    assert_eq!(hashlink, names(haxe::Runtime::Rayzor));
    for class in [
        "Window",
        "Monitor",
        "WindowAttributes",
        "Event",
        "KeyCode",
        "Key",
    ] {
        assert!(hashlink.contains(&format!("window/{class}.hx")), "{class}");
    }
    let files = xwindow_bindgen::haxe(haxe::Runtime::HashLink).unwrap();
    let window = &files
        .iter()
        .find(|f| f.path == "window/Window.hx")
        .unwrap()
        .source;
    assert!(window.contains("@:hlNative(\"xwindow\", \"window_poll_variant\")"));
    assert!(window.contains("public inline function poll():Event {"));
}
