#[cfg(target_os = "windows")]
fn main() {
    // Embed the full-bleed rounded ICO (see tools/gen_windows_icon.py) so the
    // taskbar and file explorer pick it up. `winres` falls back to a no-op
    // when the Windows SDK resource compiler (`rc.exe`) is not on `PATH`.
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon-windows.ico");
    res.compile().expect("failed to compile Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
