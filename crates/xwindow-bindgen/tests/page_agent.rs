//! The page agent end to end, under Node with a stand-in DOM: commands the
//! generated Rust encodes reach the page's objects through the mailbox, and
//! events the page fires come back through the queue, which the generated
//! Rust decodes. Needs `rustc` and `node`.

use std::path::Path;
use std::process::Command;

/// Encodes a batch of commands, or decodes the queue's records from the
/// memory the harness wrote, by the block's layout: the mailbox at 0, the
/// queue at 24, the ring at 1024.
const PROGRAM: &str = r#"
#[allow(dead_code, non_camel_case_types, clippy::all)]
mod wire {
    include!("wire.rs");
}
use std::sync::atomic::Ordering::SeqCst;
use wire::*;

fn main() {
    let mode = std::env::args().nth(1).unwrap();
    if mode == "encode" {
        let mut e = Encoder::new();
        e.document_set_title(Handle(2), &"Hello".to_owned());
        e.css_style_declaration_set_property(Handle(4), &"cursor".to_owned(), &"pointer".to_owned(), &None);
        e.xw_agent_set_size(Handle(5), &400.0, &250.0);
        e.xw_agent_set_visible(Handle(5), &false);
        std::fs::write("batch.bin", &e.bytes).unwrap();
        return;
    }
    let memory = std::fs::read("memory.bin").unwrap();
    let word = |at: usize| u32::from_le_bytes(memory[at..at + 4].try_into().unwrap());
    let queue = Queue::new();
    queue.tail.store(word(24) as i32, SeqCst);
    queue.head.store(word(28), SeqCst);
    queue.capacity.store(word(36), SeqCst);
    let ring = &memory[word(32) as usize..][..word(36) as usize];
    while let Some(event) = queue.take::<XwEvent>(ring) {
        let mut line = format!("{:?}", event.kind);
        for (name, value) in [
            ("device", event.device.map(|v| v.to_string())),
            ("x", event.x.map(|v| v.to_string())),
            ("y", event.y.map(|v| v.to_string())),
            ("width", event.width.map(|v| v.to_string())),
            ("height", event.height.map(|v| v.to_string())),
            ("scale", event.scale.map(|v| v.to_string())),
            ("on", event.on.map(|v| v.to_string())),
            ("button", event.button.map(|v| v.to_string())),
            ("code", event.code.clone()),
            ("key", event.key.clone()),
            ("modifiers", event.modifiers.map(|v| v.to_string())),
        ] {
            if let Some(value) = value {
                line.push_str(&format!(" {name}={value}"));
            }
        }
        println!("{line}");
    }
}
"#;

const HARNESS: &str = r#"
import { readFileSync, writeFileSync } from "node:fs";

// A page with just enough DOM for the agent.
class Element extends EventTarget {
  constructor() {
    super();
    this.style = { setProperty(name, value) { this[name] = value; } };
    this.tabIndex = -1;
    this.hidden = true;
  }
  // Laid out at its CSS size, or 320 by 200.
  getBoundingClientRect() {
    return { left: 0, top: 0, width: parseFloat(this.style.width) || 320, height: parseFloat(this.style.height) || 200 };
  }
  focus() { document.activeElement = this; }
  contains(other) { return other === this; }
}
const page = new EventTarget();
for (const name of ["addEventListener", "removeEventListener", "dispatchEvent"]) {
  globalThis[name] = page[name].bind(page);
}
globalThis.document = Object.assign(new EventTarget(), {
  title: "",
  visibilityState: "visible",
  activeElement: null,
  body: { appendChild() {} },
  createElement: () => new Element(),
});
globalThis.devicePixelRatio = 2;
globalThis.screen = { width: 1920, height: 1080 };
globalThis.matchMedia = (query) => ({ matches: false, media: query, addEventListener() {} });
globalThis.ResizeObserver = class { observe() {} };
globalThis.requestAnimationFrame = (frame) => setTimeout(frame, 1);
Object.defineProperty(globalThis, "navigator", { value: { userActivation: { isActive: false } } });
const fire = (type, props) => canvas.dispatchEvent(Object.assign(new Event(type), props));

const memory = { buffer: new SharedArrayBuffer(1 << 20) };
const words = new Int32Array(memory.buffer);
// The queue: tail, head, ring address, capacity, dropped.
words.set([0, 0, 1024, 1 << 16, 0], 24 >> 2);

const canvas = new Element();
const { start } = await import("./xwindow.mjs");
start({ memory, address: 0, canvas });

fire("pointermove", { pointerId: 1, pointerType: "mouse", offsetX: 10, offsetY: 20, movementX: 0, movementY: 0, pressure: 0 });
fire("pointerdown", { pointerId: 1, pointerType: "mouse", button: 2, offsetX: 10, offsetY: 20, pressure: 0.5 });
fire("keydown", { code: "KeyB", key: "b", location: 0, repeat: false });
fire("keydown", { code: "KeyA", key: "a", location: 0, repeat: false, shiftKey: true });
Atomics.store(words, 44 >> 2, 1); // a redraw, asked for
await new Promise((resolve) => setTimeout(resolve, 20));

// A batch, handed over as a program does.
const batch = readFileSync("batch.bin");
new Uint8Array(memory.buffer, 1 << 17, batch.length).set(batch);
words.set([1 << 17, batch.length], 2);
Atomics.add(words, 0, 1);
Atomics.notify(words, 0);
while (Atomics.load(words, 1) !== 1) await new Promise((resolve) => setTimeout(resolve, 1));

writeFileSync("memory.bin", new Uint8Array(memory.buffer));
console.log(JSON.stringify({
  title: document.title,
  cursor: canvas.style.cursor,
  width: canvas.style.width,
  height: canvas.style.height,
  hidden: canvas.hidden,
  tabIndex: canvas.tabIndex,
}));
process.exit(0);
"#;

#[test]
fn commands_reach_the_page_and_its_events_come_back() {
    let wire = xwindow_bindgen::browser_wire().unwrap();
    let dir = std::env::temp_dir().join(format!("xwindow-page-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("wire.rs"), &wire.rust).unwrap();
    std::fs::write(dir.join(xwindow_backend::WIRE_MODULE), &wire.js).unwrap();
    std::fs::write(
        dir.join(xwindow_backend::AGENT_MODULE),
        xwindow_backend::AGENT,
    )
    .unwrap();
    std::fs::write(dir.join("main.rs"), PROGRAM).unwrap();
    std::fs::write(dir.join("harness.mjs"), HARNESS).unwrap();

    run(
        &dir,
        Command::new("rustc").args(["--edition", "2024", "-O", "main.rs", "-o", "program"]),
    );
    run(&dir, Command::new(dir.join("program")).arg("encode"));
    let page = run(&dir, Command::new("node").arg("harness.mjs"));
    let events = run(&dir, Command::new(dir.join("program")).arg("decode"));
    std::fs::remove_dir_all(&dir).ok();

    assert_eq!(
        page.trim(),
        r#"{"title":"Hello","cursor":"pointer","width":"400px","height":"250px","hidden":true,"tabIndex":0}"#
    );
    // The window as it is when the agent starts, then what the page fired,
    // in physical pixels at a scale of 2, then the size the batch set. A key
    // with no modifiers held changes none.
    assert_eq!(
        events.trim(),
        [
            "Screen width=1920 height=1080",
            "Resized width=640 height=400 scale=2",
            "Focused on=false",
            "ThemeChanged on=false",
            "CursorMoved device=1 x=20 y=40",
            "MouseInput device=1 on=true button=2",
            "KeyboardInput on=true code=KeyB key=b",
            "KeyboardInput on=true code=KeyA key=a",
            "ModifiersChanged modifiers=1",
            "RedrawRequested",
            "Resized width=800 height=500 scale=2",
        ]
        .join("\n")
    );
}

fn run(dir: &Path, command: &mut Command) -> String {
    let out = command.current_dir(dir).output().expect("the tool runs");
    assert!(
        out.status.success(),
        "{:?}: {}{}",
        command,
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8_lossy(&out.stdout).into_owned()
}
