//! The application's icon. macOS shows one in the Dock for the program, not for its windows, so a window's icon, which winit
//! sets on the platforms where a window has one, changes nothing there: a program that is not an app bundle, such as one a
//! runtime runs, shows the runtime's icon. `set` changes the application's, from the same RGBA pixels. Off macOS it does nothing.

#[cfg(target_os = "macos")]
mod macos;

/// Makes `rgba`, `width` by `height` pixels of red, green, blue and alpha, the application's icon; no pixels restore the default.
/// On macOS this has to be called on the main thread, where a window is made; it is an error anywhere else.
pub fn set(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    check(rgba, width, height)?;
    platform(rgba, width, height)
}

/// What an icon of `width` by `height` pixels holds, or why `rgba` is not that.
fn check(rgba: &[u8], width: u32, height: u32) -> Result<(), String> {
    if rgba.is_empty() {
        return Ok(());
    }
    let expected = u64::from(width) * u64::from(height) * 4;
    if width == 0 || height == 0 || rgba.len() as u64 != expected {
        return Err(format!(
            "appicon: an icon is width by height RGBA pixels, and {} bytes are not {width} by {height}",
            rgba.len()
        ));
    }
    Ok(())
}

#[cfg(target_os = "macos")]
use macos::set as platform;

#[cfg(not(target_os = "macos"))]
fn platform(_: &[u8], _: u32, _: u32) -> Result<(), String> {
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_pixels_are_the_default() {
        assert!(check(&[], 0, 0).is_ok());
    }

    #[test]
    fn pixels_match_their_size() {
        assert!(check(&[0; 16], 2, 2).is_ok());
        assert!(check(&[0; 12], 2, 2).is_err());
        assert!(check(&[0; 16], 0, 4).is_err());
        assert!(check(&[0; 16], 4, 0).is_err());
    }
}
