use gpui::{InteractiveElement, MouseButton};

/// Let presses reach root focus handling, and consume releases after control
/// click actions run so they cannot become titlebar double clicks.
pub(super) fn consume_control_mouse_events<E: InteractiveElement>(element: E) -> E {
    element.on_mouse_up(MouseButton::Left, |_, _, cx| cx.stop_propagation())
}

/// Reuse gpui-component's TitleBar drag/double-click handlers on our existing
/// empty drag regions. Controls remain outside these regions, preserving the
/// content root's focus behavior and preventing button-originated drags.
pub(super) fn drag_region<E: gpui::StatefulInteractiveElement>(
    element: E,
    window: &mut gpui::Window,
    cx: &mut gpui::App,
    key: &'static str,
) -> E {
    let element = element.occlude();
    #[cfg(target_os = "macos")]
    {
        let state = window.use_keyed_state(key, cx, |_, _| DragState { should_move: false });
        element
            .on_mouse_down_out(window.listener_for(&state, |state, _, _, _| {
                state.should_move = false;
            }))
            .on_mouse_down(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = true;
                }),
            )
            .on_mouse_up(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                window.listener_for(&state, |state, _, _, _| {
                    state.should_move = false;
                }),
            )
            .on_mouse_move(window.listener_for(
                &state,
                |state, event: &gpui::MouseMoveEvent, window, _| {
                    if std::mem::take(&mut state.should_move)
                        && event.pressed_button == Some(MouseButton::Left)
                    {
                        super::mac_titlebar::start_window_move(window);
                    }
                },
            ))
            .on_click(|event, window, _| {
                if event.click_count() == 2 {
                    window.titlebar_double_click();
                }
            })
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (window, cx, key);
        element
    }
}

/// Compatibility equivalent of WindowOptions::app_owns_titlebar_drag=true.
pub(super) fn app_owned<E>(element: E, window: &gpui::Window) -> E {
    #[cfg(target_os = "macos")]
    super::mac_titlebar::take_ownership(window);
    #[cfg(not(target_os = "macos"))]
    let _ = window;
    element
}

#[cfg(target_os = "macos")]
struct DragState {
    should_move: bool,
}

#[cfg(target_os = "macos")]
impl gpui::Render for DragState {
    fn render(
        &mut self,
        _: &mut gpui::Window,
        _: &mut gpui::Context<Self>,
    ) -> impl gpui::IntoElement {
        gpui::Empty
    }
}

#[cfg(test)]
pub(super) mod tests {
    use std::cell::Cell;
    use std::rc::Rc;

    use gpui::StatefulInteractiveElement;

    use gpui::{
        AnyElement, AppContext, Context, Entity, FocusHandle, Focusable, InteractiveElement,
        IntoElement, MouseButton, MouseDownEvent, MouseUpEvent, ParentElement, Pixels, Point,
        Render, Styled, TestAppContext, Window, WindowControlArea, div, point,
        prelude::FluentBuilder, px,
    };

    use super::super::{appearance, text_field::TextField};

    struct Toolbar {
        build: Box<dyn Fn(Rc<Cell<usize>>) -> AnyElement>,
        clicks: Rc<Cell<usize>>,
        propagated: Rc<Cell<usize>>,
        background_clicks: Rc<Cell<usize>>,
        focus: FocusHandle,
        input: Entity<TextField>,
    }

    impl Render for Toolbar {
        fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            let down = self.propagated.clone();
            let up = self.propagated.clone();
            let background_clicks = self.background_clicks.clone();
            div()
                .size_full()
                .flex()
                .flex_col()
                .items_start()
                .track_focus(&self.focus)
                .on_mouse_down(MouseButton::Left, move |_, _, _| down.set(down.get() + 1))
                .on_mouse_up(MouseButton::Left, move |_, _, _| up.set(up.get() + 1))
                .child(
                    div()
                        .w_full()
                        .flex()
                        .child((self.build)(self.clicks.clone()))
                        .child(
                            div()
                                .id("test-drag-region")
                                .w(px(400.))
                                .h(px(32.))
                                .window_control_area(WindowControlArea::Drag)
                                .map(|region| {
                                    super::drag_region(region, window, cx, "test-drag-region")
                                })
                                .on_click(move |_, _, _| {
                                    background_clicks.set(background_clicks.get() + 1)
                                }),
                        ),
                )
                .child(self.input.clone())
        }
    }

    /// Exercise real controls through GPUI dispatch, preserving root focus and
    /// native caption suppression when an input was focused before the click.
    pub(crate) fn assert_consumes_clicks(
        cx: &mut TestAppContext,
        build: impl Fn(Rc<Cell<usize>>) -> AnyElement + 'static,
        position: Point<Pixels>,
        activates: bool,
    ) {
        let clicks = Rc::new(Cell::new(0));
        let propagated = Rc::new(Cell::new(0));
        let background_clicks = Rc::new(Cell::new(0));
        cx.update(|cx| cx.set_global(appearance::resolve(&Default::default(), &[])));
        let (view, cx) = cx.add_window_view(|_, cx| Toolbar {
            build: Box::new(build),
            clicks: clicks.clone(),
            propagated: propagated.clone(),
            background_clicks: background_clicks.clone(),
            focus: cx.focus_handle(),
            input: cx.new(|cx| TextField::new("Search", false, cx)),
        });
        let (focus, input) = cx.read(|cx| {
            let view = view.read(cx);
            (view.focus.clone(), view.input.clone())
        });
        let input_focus = cx.read(|cx| input.read(cx).focus_handle(cx));
        cx.update(|window, _| window.focus(&input_focus));
        cx.run_until_parked();
        cx.simulate_input("q");
        assert_eq!(cx.read(|cx| input.read(cx).content().to_owned()), "q");
        for click_count in 1..=3 {
            cx.update(|window, _| window.focus(&input_focus));
            cx.run_until_parked();
            cx.simulate_event(MouseDownEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count,
                first_mouse: false,
            });
            cx.update(|window, _| {
                assert!(
                    focus.is_focused(window),
                    "control press must restore root keyboard focus"
                );
                assert!(
                    window.default_prevented(),
                    "focused root must consume native caption press"
                );
            });
            assert_eq!(
                propagated.get(),
                click_count,
                "press {click_count} must reach root focus handling"
            );
            cx.simulate_event(MouseUpEvent {
                button: MouseButton::Left,
                position,
                modifiers: Default::default(),
                click_count,
            });
            assert_eq!(
                propagated.get(),
                click_count,
                "release {click_count} reached titlebar handling"
            );
            assert_eq!(clicks.get(), if activates { click_count } else { 0 });
            cx.simulate_input("x");
            assert_eq!(
                cx.read(|cx| input.read(cx).content().to_owned()),
                "q",
                "typing after a control click must not edit the input"
            );
            cx.run_until_parked();
        }
        assert_eq!(
            background_clicks.get(),
            0,
            "controls must not reach titlebar click handling"
        );
        // Events on the empty titlebar remain available to window handling.
        cx.simulate_event(MouseDownEvent {
            button: MouseButton::Left,
            position: point(px(300.), px(10.)),
            modifiers: Default::default(),
            click_count: 2,
            first_mouse: false,
        });
        assert_eq!(
            propagated.get(),
            3,
            "drag area must occlude the root focus handler"
        );
        cx.update(|window, _| {
            assert!(
                !window.default_prevented(),
                "empty drag area must retain native caption handling"
            )
        });
        cx.simulate_event(MouseUpEvent {
            button: MouseButton::Left,
            position: point(px(300.), px(10.)),
            modifiers: Default::default(),
            click_count: 2,
        });
        assert_eq!(
            background_clicks.get(),
            1,
            "empty titlebar double click must be handled once"
        );
    }
}
