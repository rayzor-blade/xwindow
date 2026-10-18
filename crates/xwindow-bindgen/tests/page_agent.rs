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
            // A pasted piece's base64, by its length.
            ("base64", event.text.clone().filter(|_| event.kind == XwEventKind::PasteData).map(|t| t.len().to_string())),
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
    this.dataset = {};
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
// The paste shortcut, which the paste stands for, then the paste: text, HTML
// and an image larger than a piece.
canvas.focus();
fire("keydown", { code: "KeyV", key: "v", location: 0, repeat: false, ctrlKey: true });
fire("keyup", { code: "KeyV", key: "v", location: 0, repeat: false, ctrlKey: true });
const image = new Uint8Array(30000).fill(7);
const clipboardData = {
  types: ["text/plain", "text/html", "Files"],
  getData: (type) => ({ "text/plain": "hi", "text/html": "<b>hi</b>" })[type] ?? "",
  files: [{ type: "image/png", arrayBuffer: async () => image.buffer }],
};
document.dispatchEvent(Object.assign(new Event("paste"), { clipboardData }));
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
            "PasteData on=true key=text/plain base64=4",
            "PasteData on=true key=text/html base64=12",
            "PasteData on=true key=image/png base64=16384",
            "PasteData on=false key=image/png base64=16384",
            "PasteData on=false key=image/png base64=7232",
            "Paste",
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

#[test]
fn a_program_that_reads_nothing_loses_no_press_and_no_release() {
    let wire = xwindow_bindgen::browser_wire().unwrap();
    let dir = std::env::temp_dir().join(format!("xwindow-flood-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("wire.rs"), &wire.rust).unwrap();
    std::fs::write(dir.join(xwindow_backend::WIRE_MODULE), &wire.js).unwrap();
    std::fs::write(
        dir.join(xwindow_backend::AGENT_MODULE),
        xwindow_backend::AGENT,
    )
    .unwrap();
    // The program that decodes takes the file to read.
    std::fs::write(
        dir.join("main.rs"),
        PROGRAM.replace(
            "std::fs::read(\"memory.bin\")",
            "std::fs::read(std::env::args().nth(2).unwrap())",
        ),
    )
    .unwrap();
    run(
        &dir,
        Command::new("rustc").args(["--edition", "2024", "-O", "main.rs", "-o", "program"]),
    );
    let setup = &HARNESS[..HARNESS.find("fire(\"pointermove\"").unwrap()];
    // A ring of 8 KB, which the flood fills many times over.
    let setup = setup.replace("1 << 16", "1 << 13");
    let harness = format!(
        "{setup}\n{}",
        r#"
const mouse = (extra) => ({ pointerId: 1, pointerType: "mouse", offsetY: 5, movementX: 0, movementY: 0, pressure: 0, ...extra });
// The program reads nothing: moves, a press, moves, a release, wheel turns, and a redraw asked for.
for (let i = 0; i < 1000; i++) fire("pointermove", mouse({ offsetX: i }));
fire("pointerdown", mouse({ button: 0, offsetX: 1000, pressure: 0.5 }));
for (let i = 0; i < 1000; i++) fire("pointermove", mouse({ offsetX: 1000 + i }));
fire("pointerup", mouse({ button: 0, offsetX: 2000 }));
for (let i = 0; i < 100; i++) fire("wheel", { deltaMode: 0, deltaX: 0, deltaY: 10, ctrlKey: false, preventDefault() {} });
Atomics.store(words, 44 >> 2, 1);
await new Promise((resolve) => setTimeout(resolve, 20));
writeFileSync("full.bin", new Uint8Array(memory.buffer));
// The program reads all of it, and the agent posts what it held at its next frame.
Atomics.store(words, 28 >> 2, Atomics.load(words, 24 >> 2));
await new Promise((resolve) => setTimeout(resolve, 20));
writeFileSync("after.bin", new Uint8Array(memory.buffer));
process.exit(0);
"#
    );
    std::fs::write(dir.join("harness.mjs"), harness).unwrap();
    run(&dir, Command::new("node").arg("harness.mjs"));
    let full = run(&dir, Command::new(dir.join("program")).args(["decode", "full.bin"]));
    let after = run(&dir, Command::new(dir.join("program")).args(["decode", "after.bin"]));
    std::fs::remove_dir_all(&dir).ok();

    let all: Vec<&str> = full.lines().chain(after.lines()).collect();
    let at = |what: &str| all.iter().position(|l| l.starts_with(what));
    let press = at("MouseInput device=1 on=true").expect("the press arrived");
    let release = at("MouseInput device=1 on=false").expect("the release arrived");
    assert!(press < release, "{all:#?}");
    // Where the pointer was when it was pressed, and where it ended.
    let moved = |line: &str| line.starts_with("CursorMoved");
    let before = all[..press].iter().rev().find(|l| moved(l)).expect("a move before the press");
    assert!(before.contains("x=1998 "), "{before}");
    let last = all.iter().rev().find(|l| moved(l)).expect("a move at the end");
    assert!(last.contains("x=3998 "), "{last}");
    // Far fewer moves than were made, and a redraw.
    assert!(all.iter().filter(|l| moved(l)).count() < 400, "{}", all.len());
    assert!(all.iter().any(|l| *l == "RedrawRequested"), "{all:#?}");
}

const GAMEPAD_PROGRAM: &str = r#"
#[allow(dead_code,non_camel_case_types,clippy::all)] mod wire {include!("wire.rs");}
use wire::*;
use std::sync::atomic::Ordering::SeqCst;
fn main() {
 let mode=std::env::args().nth(1).unwrap();
 if mode=="encode" {
  let mut e=Encoder::new(); e.xw_agent_enable_gamepads(Handle(5)); std::fs::write("batch.bin",&e.bytes).unwrap();
  let mut e=Encoder::new(); e.xw_agent_rumble_gamepad(Handle(5),&3,&0.75,&0.25,&0.125); std::fs::write("rumble.bin",&e.bytes).unwrap(); return;
 }
 let memory=std::fs::read("memory.bin").unwrap();
 let word=|at:usize|u32::from_le_bytes(memory[at..at+4].try_into().unwrap());
 let q=Queue::new(); q.tail.store(word(24) as i32,SeqCst);q.head.store(word(28),SeqCst);q.capacity.store(word(36),SeqCst);
 let ring=&memory[word(32) as usize..][..word(36) as usize];
 while let Some(e)=q.take::<XwEvent>(ring) {
  match e.kind {
   XwEventKind::GamepadStatus=>println!("status {:?}",e.on),
   XwEventKind::GamepadSnapshot=>println!("snapshot {:?} {:?} {:?} {:?} {:?} {:?}",e.device,e.gamepad_index,e.gamepad_buttons,e.gamepad_axes,e.gamepad_pressed,e.gamepad_rumble),
   XwEventKind::GamepadDisconnected=>println!("disconnect {:?}",e.device),
   XwEventKind::GamepadRumble=>println!("rumble {:?}",e.on),
   _=>(),
  }
 }
}
"#;

#[test]
fn gamepad_samples_and_rumble_cross_the_declared_wire() {
    let wire = xwindow_bindgen::browser_wire().unwrap();
    let dir = std::env::temp_dir().join(format!("xwindow-gamepads-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("wire.rs"), &wire.rust).unwrap();
    std::fs::write(dir.join(xwindow_backend::WIRE_MODULE), &wire.js).unwrap();
    std::fs::write(
        dir.join(xwindow_backend::AGENT_MODULE),
        xwindow_backend::AGENT,
    )
    .unwrap();
    std::fs::write(dir.join("program.rs"), GAMEPAD_PROGRAM).unwrap();
    run(
        &dir,
        Command::new("rustc").args(["--edition=2024", "program.rs", "-o", "program"]),
    );
    run(&dir, Command::new(dir.join("program")).arg("encode"));
    let setup = &HARNESS[..HARNESS.find("fire(\"pointermove\"").unwrap()];
    let harness = format!(
        "{setup}\n{}",
        r#"
const calls=[];
const actuator={ effects:["dual-rumble"],playEffect:(type,params)=>{calls.push({type,params});return Promise.resolve("complete");} };
const makePad=()=>({index:3,connected:true,id:"test pad",mapping:"standard",axes:[0.125,-0.5,0,0],buttons:Array.from({length:17},(_,i)=>({value:i===6?0.25:0,pressed:false})),vibrationActuator:actuator});
let pad=makePad(); navigator.getGamepads=()=>[null,null,null,pad];
const batch=async(file)=> {
 const bytes=readFileSync(file);new Uint8Array(memory.buffer,1<<17,bytes.length).set(bytes);words.set([1<<17,bytes.length],2);
 const serial=Atomics.add(words,0,1)+1;Atomics.notify(words,0);
 while(Atomics.load(words,1)!==serial) await new Promise(r=>setTimeout(r,1));
};
await batch("batch.bin");
pad.buttons[0]={value:1,pressed:true};pad.axes[1]=0.375;
await new Promise(r=>setTimeout(r,10));
await batch("rumble.bin");
pad.connected=false;page.dispatchEvent(Object.assign(new Event("gamepaddisconnected"),{gamepad:pad}));
await new Promise(r=>setTimeout(r,10));
pad=makePad();page.dispatchEvent(Object.assign(new Event("gamepadconnected"),{gamepad:pad}));
await new Promise(r=>setTimeout(r,10));
const unmapped={...makePad(),index:5,id:"unmapped pad",mapping:"",vibrationActuator:null};
navigator.getGamepads=()=>[null,null,null,pad,null,unmapped];
await new Promise(r=>setTimeout(r,10));
writeFileSync("memory.bin",new Uint8Array(memory.buffer));console.log(JSON.stringify(calls));process.exit(0);
"#
    );
    std::fs::write(dir.join("harness.mjs"), harness).unwrap();
    let output = run(&dir, Command::new("node").arg("harness.mjs"));
    let calls: serde_json::Value = serde_json::from_str(output.trim()).unwrap();
    assert_eq!(
        calls,
        serde_json::json!([{"type":"dual-rumble","params":{"duration":125,"strongMagnitude":0.75,"weakMagnitude":0.25}}])
    );
    let events = run(&dir, Command::new(dir.join("program")).arg("decode"));
    assert!(events.contains("status Some(true)"), "{events}");
    assert!(
        events.contains("snapshot Some(1) Some(3) Some([0.0, 0.0, 0.0, 0.0, 0.0, 0.0, 0.25"),
        "{events}"
    );
    assert!(
        events.contains("Some([0.125, -0.5, 0.0, 0.0]) Some(0) Some(true)"),
        "{events}"
    );
    assert!(
        events.contains("Some([0.125, 0.375, 0.0, 0.0]) Some(1) Some(true)"),
        "{events}"
    );
    assert!(events.contains("rumble Some(true)"), "{events}");
    assert!(events.contains("disconnect Some(1)"), "{events}");
    assert!(events.contains("snapshot Some(2) Some(3)"), "{events}");
    assert!(
        events.contains("snapshot Some(3) Some(5) Some([]) Some([]) Some(0) Some(false)"),
        "{events}"
    );
    std::fs::remove_dir_all(dir).unwrap();
}
