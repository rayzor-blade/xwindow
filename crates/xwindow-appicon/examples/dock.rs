//! Sets the application's icon to a gradient and keeps the program running to look at: `cargo run -p xwindow-appicon --example dock -- 8`
//! (the seconds to stay up, 8 by default). On macOS it reads the icon back; elsewhere it only calls `set`.

fn main() {
    let seconds: u64 = std::env::args()
        .nth(1)
        .and_then(|s| s.parse().ok())
        .unwrap_or(8);
    let size = 256u32;
    let mut pixels = Vec::with_capacity((size * size * 4) as usize);
    for y in 0..size {
        for x in 0..size {
            let (dx, dy) = (x as f32 - 127.5, y as f32 - 127.5);
            let inside = dx * dx + dy * dy < 120.0 * 120.0;
            pixels.extend_from_slice(&[
                (x as u8),
                90,
                (255 - y as u8),
                if inside { 255 } else { 0 },
            ]);
        }
    }
    #[cfg(target_os = "macos")]
    {
        use objc2_app_kit::{NSApplication, NSApplicationActivationPolicy};
        use objc2_foundation::{MainThreadMarker, NSDate, NSRunLoop};
        let mtm = MainThreadMarker::new().expect("main thread");
        let app = NSApplication::sharedApplication(mtm);
        app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
        xwindow_appicon::set(&pixels, size, size).expect("set");
        let read = unsafe { app.applicationIconImage() }.expect("an icon image");
        let read_size = unsafe { read.size() };
        println!(
            "application icon is {} by {} points",
            read_size.width, read_size.height
        );
        assert_eq!(
            (read_size.width as u32, read_size.height as u32),
            (size, size)
        );
        unsafe {
            app.finishLaunching();
            #[allow(deprecated)]
            app.activateIgnoringOtherApps(true);
            let end = NSDate::dateWithTimeIntervalSinceNow(seconds as f64);
            NSRunLoop::currentRunLoop().runUntilDate(&end);
        }
        xwindow_appicon::set(&[], 0, 0).expect("restore");
        println!("restored the default");
    }
    #[cfg(not(target_os = "macos"))]
    {
        xwindow_appicon::set(&pixels, size, size).expect("set");
        println!("not macOS: nothing to show, {seconds} s ignored");
    }
}
