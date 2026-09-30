//! Native AppKit regression probe (not GPUI's test platform).
//! Run `cargo run --example mac_titlebar_probe`; `-- --baseline` disables the
//! bridge and demonstrates that native titlebar clicks miss the GPUI button.

#[cfg(target_os = "macos")]
#[path = "../src/ui/mac_titlebar.rs"]
mod mac_titlebar;
#[cfg(target_os = "macos")]
#[path = "../src/ui/titlebar.rs"]
mod titlebar;
#[cfg(target_os = "macos")]
mod theme {
    pub const TITLEBAR_HEIGHT: gpui::Pixels = gpui::px(32.);
}

#[cfg(target_os = "macos")]
fn main() {
    use gpui::{
        App, AppContext, Application, Context, InteractiveElement, IntoElement, ParentElement,
        Render, StatefulInteractiveElement, Styled, Timer, TitlebarOptions, Window, WindowBounds,
        WindowOptions, div, point, prelude::FluentBuilder, px, rgb, size,
    };
    use objc2::MainThreadMarker;
    use objc2_app_kit::{NSApplication, NSEvent, NSEventModifierFlags, NSEventType, NSView};
    use objc2_foundation::NSPoint;
    use raw_window_handle::{HasWindowHandle, RawWindowHandle};
    use std::{cell::Cell, rc::Rc, time::Duration};

    struct Probe {
        baseline: bool,
        clicks: Rc<Cell<usize>>,
        background_clicks: Rc<Cell<usize>>,
        left: f32,
    }
    impl Render for Probe {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let clicks = self.clicks.clone();
            let background_clicks = self.background_clicks.clone();
            div().size_full().bg(rgb(0xffffff)).child(
                div()
                    .id("probe-titlebar")
                    .when(!self.baseline, |bar| titlebar::app_owned(bar, window))
                    .h(theme::TITLEBAR_HEIGHT)
                    .flex()
                    .child(div().w(px(self.left)).flex_none())
                    .child(
                        div()
                            .id("probe-button")
                            .size(px(28.))
                            .map(titlebar::consume_control_mouse_events)
                            .bg(rgb(0xaaccff))
                            .on_click(move |_, _, _| clicks.set(clicks.get() + 1)),
                    )
                    .child(
                        div()
                            .id("probe-drag-region")
                            .h_full()
                            .flex_1()
                            .map(|region| {
                                titlebar::drag_region(region, window, cx, "probe-drag-region")
                            })
                            .on_click(move |_, _, _| {
                                background_clicks.set(background_clicks.get() + 1)
                            }),
                    ),
            )
        }
    }

    let baseline = std::env::args().any(|argument| argument == "--baseline");
    Application::new().run(move |cx: &mut App| {
        // An ordinary native-titlebar window must keep GPUI's original class.
        let ordinary = cx.open_window(WindowOptions { focus: false, ..Default::default() },
            |_, cx| cx.new(|_| Probe { baseline: true, clicks: Rc::new(Cell::new(0)),
                background_clicks: Rc::new(Cell::new(0)), left: 100. })).unwrap();
        let mut probes = Vec::new();
        for name in ["main", "diff"] {
        let clicks = Rc::new(Cell::new(0));
        let probe_clicks = clicks.clone();
        let background_clicks = Rc::new(Cell::new(0));
        let probe_background_clicks = background_clicks.clone();
        let handle = cx.open_window(
            WindowOptions {
                focus: false,
                window_bounds: Some(WindowBounds::Windowed(gpui::Bounds::new(
                    point(px(100.), px(100.)), size(px(400.), px(250.)),
                ))),
                titlebar: Some(TitlebarOptions {
                    title: Some(format!("ReviewFox {name} titlebar probe").into()),
                    appears_transparent: true,
                    ..Default::default()
                }),
                ..Default::default()
            },
            |_, cx| cx.new(|_| Probe { baseline, clicks: probe_clicks, background_clicks: probe_background_clicks, left: 100. }),
        ).unwrap();
        probes.push((name, handle, clicks, background_clicks));
        }
        cx.spawn(async move |cx| {
            Timer::after(Duration::from_millis(300)).await;
            let ordinary_unchanged = ordinary.update(cx, |_, window, _| {
                let raw = HasWindowHandle::window_handle(window).unwrap();
                let RawWindowHandle::AppKit(raw) = raw.as_raw() else { unreachable!() };
                // SAFETY: borrowed native view of this live window.
                let view = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
                !view.class().name().to_bytes().starts_with(b"ReviewFoxAppOwned")
            }).unwrap();
            println!("native ordinary titlebar: original GPUI class preserved={ordinary_unchanged}");
            let mut all_passed = ordinary_unchanged;
            for (name, handle, clicks, background_clicks) in &probes {
            let (native_window, height, frame) = handle.update(cx, |_, window, _| {
                let raw = HasWindowHandle::window_handle(window).unwrap();
                let RawWindowHandle::AppKit(raw) = raw.as_raw() else { unreachable!() };
                // SAFETY: this is the live GPUI view for our probe window.
                let view = unsafe { raw.ns_view.cast::<NSView>().as_ref() };
                let native_window = view.window().unwrap();
                let frame = native_window.frame();
                (native_window, view.bounds().size.height, frame)
            }).unwrap();
            let app = NSApplication::sharedApplication(MainThreadMarker::new().unwrap());
            let send = |event_type, location, click_count| {
                    let event = NSEvent::mouseEventWithType_location_modifierFlags_timestamp_windowNumber_context_eventNumber_clickCount_pressure(
                        event_type, location,
                        NSEventModifierFlags::empty(), 0., native_window.windowNumber(),
                        None, click_count, click_count, 1.,
                    ).unwrap();
                    // Dispatch outside an App update so GPUI can re-enter its
                    // normal native event callback without a borrow conflict.
                    app.sendEvent(&event);
            };
            for click_count in 1..=3 {
                for event_type in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                    send(event_type, NSPoint::new(110., height - 14.), click_count);
                }
            }
            let mut passed = clicks.get() == 3 && native_window.frame() == frame && background_clicks.get() == 0;
            println!("native {name} titlebar: clicks={} expected=3, frame_unchanged={}, movable={}, baseline={baseline}",
                clicks.get(), native_window.frame() == frame, native_window.isMovable());
            if passed {
                send(NSEventType::LeftMouseDown, NSPoint::new(110., height - 14.), 1);
                send(NSEventType::LeftMouseDragged, NSPoint::new(220., height - 100.), 1);
                send(NSEventType::LeftMouseUp, NSPoint::new(220., height - 100.), 1);
                passed &= clicks.get() == 3;
                handle.update(cx, |probe, _, cx| { probe.left = 180.; cx.notify(); }).unwrap();
                Timer::after(Duration::from_millis(100)).await;
                send(NSEventType::LeftMouseDown, NSPoint::new(190., height - 14.), 1);
                send(NSEventType::LeftMouseUp, NSPoint::new(190., height - 14.), 1);
                passed &= clicks.get() == 4 && native_window.frame() == frame;
                println!("native {name} titlebar: outside release and moved control pass={passed}");
                for event_type in [NSEventType::LeftMouseDown, NSEventType::LeftMouseUp] {
                    send(event_type, NSPoint::new(300., height - 14.), 2);
                }
                passed &= background_clicks.get() == 1 && clicks.get() == 4;
                println!("native {name} titlebar: background double-click handled once={}", background_clicks.get() == 1);

            }
            all_passed &= passed;
            }
            cx.update(|cx| cx.quit()).unwrap();
            if !all_passed { std::process::exit(1); }
        }).detach();
    });
}

#[cfg(not(target_os = "macos"))]
fn main() {
    eprintln!("This native AppKit probe requires macOS.");
}
