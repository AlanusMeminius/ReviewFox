#[cfg(target_os = "windows")]
fn main() {
    // Embed the application icon into the PE resources so the taskbar and
    // file explorer pick it up. `winres` falls back to a no-op when the
    // Windows SDK resource compiler (`rc.exe`) is not on `PATH`.
    let mut res = winres::WindowsResource::new();
    res.set_icon("assets/icon.ico");
    res.compile().expect("failed to compile Windows resources");
}

#[cfg(not(target_os = "windows"))]
fn main() {}
