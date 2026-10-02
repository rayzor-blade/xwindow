//! The host's hook on a desktop: `host pump` lets the program pump the loop,
//! `host turns` runs it as a host does and gives the program turns.
//!
//!     cargo run -p xwindow-check --example host -- turns

mod program;

use xwindow_check::winit::event_loop::EventLoop;
use xwindow_check::{Drive, attach};

fn main() {
    let log = |s: &str| println!("host: {s}");
    let mut program = program::Program::new();
    let events = EventLoop::new().expect("an event loop");
    match std::env::args().nth(1).as_deref() {
        Some("turns") => {
            let mut turn = || program.turn(&log);
            attach(events, Drive::Turns(&mut turn)).expect("the loop runs");
        }
        Some("pump") | None => {
            attach(events, Drive::Pump).expect("the loop attaches");
            while program.turn(&log) {}
        }
        Some(other) => panic!("host pump|turns, not {other}"),
    }
    if !program.failures().is_empty() {
        for failure in program.failures() {
            eprintln!("host: FAILED: {failure}");
        }
        std::process::exit(1);
    }
    println!("host: ok");
}
