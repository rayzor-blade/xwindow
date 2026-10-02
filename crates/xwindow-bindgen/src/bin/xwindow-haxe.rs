use std::path::PathBuf;

fn main() -> Result<(), String> {
    let mut args = std::env::args_os().skip(1);
    let usage = "usage: xwindow-haxe <hashlink|ash|rayzor> <output-directory>";
    let target = args.next().and_then(|v| v.into_string().ok());
    let root = PathBuf::from(args.next().ok_or(usage)?);
    if args.next().is_some() {
        return Err(usage.into());
    }
    let runtime = match target.as_deref() {
        Some("hashlink" | "ash") => xwindow_bindgen::haxe::Runtime::HashLink,
        Some("rayzor") => xwindow_bindgen::haxe::Runtime::Rayzor,
        _ => return Err(usage.into()),
    };
    for file in xwindow_bindgen::haxe(runtime)? {
        let path = root.join(file.path);
        std::fs::create_dir_all(path.parent().expect("generated file has a parent"))
            .map_err(|e| e.to_string())?;
        std::fs::write(path, file.source).map_err(|e| e.to_string())?;
    }
    Ok(())
}
