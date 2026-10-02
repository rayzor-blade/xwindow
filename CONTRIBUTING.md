# Contributing to xwindow

The user-facing contract is the `window` namespace. Keep API changes in the
shared declaration so every runtime sees the same classes, records, enums,
events and method names.

## Repository layout

- `api/window.api.rs` is the typed API declaration.
- `api/spec/window.idl` holds the browser APIs the page agent reaches,
  verbatim from W3C's webref. `api/spec/agent.idl` holds the agent's own
  operations and the event record it posts.
- `crates/xwindow-core` has the runtime-neutral parts:
  - winit's key and cursor lists, as macros the declaration and both
    backends expand;
  - generational handles;
  - the surface contract, `raw::split` and `raw::join`.
- `crates/xwindow-bindgen` runs x-idl over the declaration for each runtime.
  It generates the browser wire and builds `xwindow-haxe`.
- `crates/xwindow-backend` carries the winit backend, the browser backend
  and the page agent, which adapters compile beneath their own ABI.
- `crates/xwindow-check` is a stand-in adapter. It compiles the generated
  model with both backends against carriers that only have a runtime's
  shapes.
- Runtime adapters stay in their host repositories. Text, buffers, errors,
  roots, allocation and exported symbols follow the host runtime's ABI.

## The declaration

x-idl reads Rust-shaped declarations:

- A trait is a resource class whose objects are integer handles.
- A struct is a record that a program fills in.
- A fieldless enum is a set of integer codes.
- An enum whose variants have named fields is a set of variants, such as
  `Event`:
  - Its fields are numbers, `bool`, `Text`, `Buffer`, `Enum<T>`, or other
    variants, which nest.
  - Its first variant, with its fields at their defaults, is what a failed
    call returns.

How each runtime takes variants:

- **Caribou** takes them whole, as nested `PluginEnum`s.
- **HashLink and Rayzor** take the variant's index. Each class keeps the
  last value of each variants type it returns. Every field, at any depth,
  has a getter over that value. A generated `read<Type>(index)` in the
  Haxe surface builds the enum from these getters.

A `Buffer` field is held as the generated `VariantBytes(Vec<u8>)`.

`xwindow_bindgen::window_api()` appends `KeyCode`, `NamedKey` and
`CursorIcon` from xwindow-core's lists. Their variants share winit's names,
so the backends convert by name, and a winit upgrade that renames a key
fails to compile. `Unrecognized` stands for a key a newer winit names.
Update the lists in `xwindow-core` when upgrading winit.

## Adapter boundary

An adapter's build script calls:

```rust
let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
xwindow_backend::install(&out)?; // xwindow_backend/{native,web}.rs, page/xwindow.mjs
fs::write(out.join("window.rs"), xwindow_bindgen::generate(Runtime::Caribou)?)?;
let wire = xwindow_bindgen::browser_wire()?;
fs::write(out.join("window_wire.rs"), wire.rust)?;
fs::write(out.join("page").join(xwindow_backend::WIRE_MODULE), wire.js)?;
if browser {
    let backend = xwindow_bindgen::web_backend(Runtime::Caribou, xwindow_backend::WEB)?;
    fs::write(out.join("window_web_backend.rs"), backend)?;
}
```

It then includes the generated model at its crate root (or under a scope,
with `install_scoped`):

- `mod backend` is `native.rs` natively.
- In a browser, `mod web` is `web.rs`, `mod backend` is the generated
  forwarder, and `mod wire` is the wire.

`crate::runtime` must provide:

- `Text`, `Buffer` (with `new`, `NULL`, `len` and `as_ptr`) and `ErrorKind`;
- `host::raise`, and in a browser `host::agent`.

The adapter depends on `xwindow-core`. Natively it also depends on `winit`
0.30 and `raw-window-handle` 0.6. `crates/xwindow-check` is the smallest
working shape.

Per runtime:

- **Caribou** uses `caribou_abi`'s carriers. The plugin is named `window`.
- **HashLink/Ash** uses an hl_abi runtime module, like hlwgpu's. Its
  primitives load from `xwindow.hdll`, and records are
  `hl.Abstract<"xwindow_*">`.
  - A `Buffer` argument arrives as the generated `HlBytes`, which has
    `haxe.io.Bytes`' layout: its length, then its data. So
    `Buffer::from_hl` takes `*mut HlBytes`.
  - The generated primitives copy `Buffer` results out. They need the
    carrier's `len` and `as_ptr`.
- **Rayzor** uses Rayzor's carriers, like rayzor-gpu's. It exports
  `export_abi_version!()` and
  `rpkg_entry!(XIDL_METHODS, xidl_runtime_symbols)`. Every symbol is
  `xwindow_*`, and every class `window::*`, matching the externs.

The native backend shares one winit event loop among all windows, on the
thread that opened the first. Pumping it sorts each window's events into
that window's queue. Raw device events go to the focused window. The loop
pumps through winit's `pump_events`, so it runs where that API does:
Windows, macOS, X11, Wayland and Android.

## Browser boundary

`web.rs` encodes each request onto the generated wire. Requests are batched
and sent through the mailbox when the program next polls or waits. The
mailbox is at the address the host's agent hook receives. The queue is 24
bytes on, and a redraw word 44 bytes on, which the agent reads every frame.

`agent.mjs` keeps the page's objects under fixed handles:

| Handle | Object |
|---|---|
| 1 | `window` |
| 2 | `document` |
| 3 | The canvas |
| 4 | The canvas's `style` |
| 5 | The agent, which implements `XwAgent` |

The agent posts `XwEvent`s through the generated `post_XwEvent`.

Keep runtime-specific wasm loading, side-module discovery, COOP/COEP serving
and canvas transfer out of xwindow.

## Validation

```sh
cargo test --workspace
cargo build -p xwindow-check --target wasm32-wasip1-threads
```

The `page_agent` test runs the real agent under Node against a stand-in DOM.
It needs `rustc` and `node`. Generate both Haxe surfaces when changing the
declaration:

```sh
cargo run -p xwindow-bindgen --bin xwindow-haxe -- ash /tmp/xwindow-ash
cargo run -p xwindow-bindgen --bin xwindow-haxe -- rayzor /tmp/xwindow-rayzor
haxe -cp /tmp/xwindow-ash -cp examples/events -main Main -hl /tmp/events.hl
```
