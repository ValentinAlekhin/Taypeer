use gpui::*;
use gpui_component::{
    button::*,
    input::{Input, InputEvent, InputState},
    *,
};
use gpui_component_assets::Assets;

pub struct Example {
    russian: bool,
    input_state: Entity<InputState>,
    display_text: SharedString,

    /// We need to keep the subscriptions alive with the Example entity.
    ///
    /// So if the Example entity is dropped, the subscriptions are also dropped.
    /// This is important to avoid memory leaks.
    _subscriptions: Vec<Subscription>,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Synthetic password / Тестовый пароль")
                .masked(true)
        });

        let _subscriptions = vec![cx.subscribe_in(&input_state, window, {
            let input_state = input_state.clone();
            move |this, _, ev: &InputEvent, _window, cx| match ev {
                InputEvent::Change => {
                    let count = input_state.read(cx).value().chars().count();
                    this.display_text = format!("Characters / Символов: {}", count).into();
                    cx.notify()
                }
                _ => {}
            }
        })];

        Self {
            russian: false,
            input_state,
            display_text: SharedString::default(),
            _subscriptions,
        }
    }
}

impl Render for Example {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_5()
            .gap_2()
            .size_full()
            .items_center()
            .child(
                Button::new("language")
                    .label("English / Русский")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.russian = !this.russian;
                        cx.notify();
                    })),
            )
            .child(if self.russian {
                "Только искусственные данные"
            } else {
                "Synthetic data only"
            })
            .child(
                div()
                    .id("entries")
                    .w_full()
                    .h_48()
                    .overflow_y_scroll()
                    .children((1..=100).map(|i| {
                        div().p_2().child(if self.russian {
                            format!("Тестовая запись {i}")
                        } else {
                            format!("Synthetic entry {i}")
                        })
                    })),
            )
            .child(Input::new(&self.input_state).mask_toggle())
            .child(self.display_text.clone())
            .child(
                Button::new("dialog")
                    .label(if self.russian {
                        "Диалог"
                    } else {
                        "Dialog"
                    })
                    .on_click(|_, window, cx| {
                        window.open_dialog(cx, |dialog, _, _| {
                            dialog
                                .title("Probe / Проверка")
                                .child("No data is saved / Данные не сохраняются")
                        });
                    }),
            )
    }
}

fn main() {
    let app = gpui_platform::application().with_assets(Assets);

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_component::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(800.), px(600.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Example::new(window, cx));
                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
