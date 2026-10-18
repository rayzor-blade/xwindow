//! `NSApplication`'s icon image, made from the pixels as a bitmap that has the application's size.

use std::ptr;

use objc2::ClassType;
use objc2::rc::Retained;
use objc2_app_kit::{NSApplication, NSBitmapImageRep, NSDeviceRGBColorSpace, NSImage};
use objc2_foundation::{MainThreadMarker, NSSize};

pub fn set(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    let mtm = MainThreadMarker::new()
        .ok_or_else(|| "appicon: the application's icon is set on the main thread".to_owned())?;
    let app = NSApplication::sharedApplication(mtm);
    if rgba.is_empty() {
        // No image is the default: the bundle's icon, or the system's for a program that has none.
        unsafe { app.setApplicationIconImage(None) };
        return Ok(());
    }
    let image = unsafe { image(rgba, width, height) }?;
    unsafe { app.setApplicationIconImage(Some(&image)) };
    Ok(())
}

/// An image of `width` by `height` points that is that many pixels, with alpha as the pixels give it.
unsafe fn image(rgba: &[u8], width: u32, height: u32) -> Result<Retained<NSImage>, String> {
    let (w, h) = (width as isize, height as isize);
    let rep = unsafe {
        NSBitmapImageRep::initWithBitmapDataPlanes_pixelsWide_pixelsHigh_bitsPerSample_samplesPerPixel_hasAlpha_isPlanar_colorSpaceName_bytesPerRow_bitsPerPixel(
            NSBitmapImageRep::alloc(),
            ptr::null_mut(),
            w,
            h,
            8,
            4,
            true,
            false,
            NSDeviceRGBColorSpace,
            w * 4,
            32,
        )
    }
    .ok_or_else(|| "appicon: no bitmap could be made for the icon".to_owned())?;
    let data = unsafe { rep.bitmapData() };
    if data.is_null() {
        return Err("appicon: the icon's bitmap has no pixels".to_owned());
    }
    unsafe { ptr::copy_nonoverlapping(rgba.as_ptr(), data, rgba.len()) };
    let image = unsafe {
        NSImage::initWithSize(NSImage::alloc(), NSSize::new(width as f64, height as f64))
    };
    unsafe { image.addRepresentation(&rep) };
    Ok(image)
}
