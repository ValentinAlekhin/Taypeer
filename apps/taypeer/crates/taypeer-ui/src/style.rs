//! Product presentation primitives shared by the new screens.

pub use crate::common::{input, tr};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState},
        *,
    },
    *,
};

/// Resolve an embedded product icon at the relative icon size.
pub fn icon(name: &str) -> Icon {
    Icon::default()
        .path(SharedString::from(format!("ui-icons/{name}.svg")))
        .size(rems(1.))
}
/// Build a default product command with the shared chrome surface and border.
/// The native Kit button retains its input, focus and accessibility behavior.
pub fn default_button(id: impl Into<ElementId>) -> Button {
    // Kit's Default uses the input surface as its border. Secondary uses the
    // border role and keeps native state paint; its colors are mapped in theme.
    Button::new(id).secondary().border_1()
}
/// Build a quiet command button with localized tooltip and accessibility label.
pub fn icon_button(id: impl Into<ElementId>, name: &str, label: &str) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .icon(icon(name))
        .tooltip(tr(label))
        .accessibility_label(tr(label))
        .h(rems(2.))
        .w(rems(2.))
}
/// Compose a labeled read-only row with a shrinkable value region.
pub fn row(label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .relative()
        .min_h(rems(2.75))
        .px(rems(1.5))
        .gap(rems(1.))
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(1.))
                .bg(cx.theme().border),
        )
        .child(
            div()
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .text_size(rems(13. / 16.))
                .line_height(rems(1.25))
                .child(tr(label)),
        )
        .child(
            div()
                .flex_1()
                .min_w_0()
                .py_1()
                .text_size(rems(13. / 16.))
                .line_height(rems(1.25))
                .child(value),
        )
        .into_any_element()
}
/// Product input paint keeps focus decoration outside the retained editor layout.
#[derive(IntoElement)]
pub struct ProductField {
    input: Input,
    state: Entity<InputState>,
    style: StyleRefinement,
    standalone: bool,
}
impl ProductField {
    /// Disable the real input without replacing its retained state.
    pub fn disabled(mut self, disabled: bool) -> Self {
        self.input = self.input.disabled(disabled);
        self
    }
    /// Set the actual control ID used by accessibility and scenarios.
    pub fn id(mut self, id: impl Into<ElementId>) -> Self {
        self.input = self.input.id(id);
        self
    }
    /// Add the product's trailing action to the real input.
    pub fn suffix(mut self, suffix: impl IntoElement) -> Self {
        self.input = self.input.suffix(suffix);
        self
    }
}
impl Styled for ProductField {
    fn style(&mut self) -> &mut StyleRefinement {
        &mut self.style
    }
}
impl RenderOnce for ProductField {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let input = self
            .input
            .bg(if self.standalone {
                cx.theme().title_bar
            } else {
                rgba(0).into()
            })
            .rounded(rems(0.25))
            .refine_style(&self.style);
        if self.standalone {
            focus_control(self.state.focus_handle(cx), input, window, cx)
        } else {
            div().relative().w_full().child(input).into_any_element()
        }
    }
}

/// Decorate a retained control without changing its padding or text geometry.
pub fn focus_control(
    focus: FocusHandle,
    control: impl IntoElement,
    window: &Window,
    cx: &App,
) -> AnyElement {
    div()
        .relative()
        .w_full()
        .child(control)
        .when(focus.is_focused(window), |el| {
            el.child(
                div()
                    .absolute()
                    .top(px(-1.))
                    .left(px(-1.))
                    .right(px(-1.))
                    .bottom(px(-1.))
                    .rounded(rems(0.25))
                    .border_2()
                    .border_color(cx.theme().ring),
            )
        })
        .into_any_element()
}

/// Render an owned input with a stable resource-key ID and accessible name.
pub fn field(state: &Entity<InputState>, label: &str) -> ProductField {
    ProductField {
        input: Input::new(state)
            .id(SharedString::from(format!("field-{label}")))
            .aria_label(tr(label))
            .appearance(false)
            .bordered(false)
            .focus_bordered(false)
            .border_0()
            .px(rems(0.625))
            .py(rems(0.375))
            .text_size(rems(13. / 16.))
            .line_height(rems(1.25))
            .h(rems(2.)),
        state: state.clone(),
        style: StyleRefinement::default(),
        standalone: true,
    }
}
/// Configure a row field without form paint or a focus outline.
pub fn row_field(state: &Entity<InputState>, label: &str) -> ProductField {
    let mut field = field(state, label).pl_0();
    field.standalone = false;
    field
}

/// An entry row reserves eight pixels between its text and trailing actions.
pub fn entry_field(state: &Entity<InputState>, label: &str) -> ProductField {
    row_field(state, label).pr(rems(0.5))
}

/// Compose the shrinkable editor input and its trailing actions.
pub fn input_control(label: &str, value: impl IntoElement) -> Stateful<Div> {
    h_flex()
        .id(SharedString::from(format!("control-{label}")))
        .gap(rems(0.5))
        .child(div().flex_1().min_w_0().child(value))
}

/// Shared underline tabs for entry details and settings.
pub fn tabs(id: &'static str, labels: &[&'static str], selected: usize, cx: &App) -> tab::TabBar {
    tab::TabBar::new(id)
        .underline()
        .selected_index(selected)
        .h(rems(2.375))
        .border_b_1()
        .border_color(cx.theme().border)
        .children(labels.iter().enumerate().map(|(index, key)| {
            tab::Tab::new()
                .when(id == "generator-tabs", |tab| {
                    tab.w(rems(if index == 0 { 6.5 } else { 11.25 }))
                        .flex_shrink_0()
                })
                .px(rems(0.5))
                .child(
                    div()
                        .when(id == "generator-tabs", |label| label.w_full())
                        .text_size(rems(13. / 16.))
                        .line_height(rems(1.25))
                        .font_weight(FontWeight::NORMAL)
                        .text_color(if index == selected {
                            cx.theme().foreground
                        } else {
                            cx.theme().muted_foreground
                        })
                        .child(tr(key)),
                )
        }))
}

/// A form label focuses its associated control without changing its value.
pub fn input_row(label: &str, focus: FocusHandle, value: impl IntoElement, cx: &App) -> AnyElement {
    let label_element = div()
        .id(SharedString::from(format!("label-{label}")))
        .w(rems(9.))
        .flex_shrink_0()
        .text_color(cx.theme().muted_foreground)
        .child(tr(label))
        .text_size(rems(13. / 16.))
        .line_height(rems(1.25))
        .on_click(move |_, window, cx| focus.focus(window, cx));
    #[cfg(any(feature = "component-rendering", feature = "ui-test-support"))]
    let label_element = label_element.test_support();
    let row = h_flex()
        .id(SharedString::from(format!("row-{label}")))
        .relative()
        .min_h(rems(2.75))
        .px(rems(1.5))
        .gap(rems(1.))
        .child(
            div()
                .absolute()
                .bottom_0()
                .left_0()
                .right_0()
                .h(px(1.))
                .bg(cx.theme().border),
        )
        .child(label_element)
        .child(
            div()
                .flex_1()
                .min_w_0()
                .py_1()
                .text_size(rems(13. / 16.))
                .line_height(rems(1.25))
                .child(value),
        );
    #[cfg(any(feature = "component-rendering", feature = "ui-test-support"))]
    let row = row.test_support();
    row.into_any_element()
}

/// Format a database-supplied RGBA value without interpreting it as a theme role.
pub fn color_text(color: Option<u32>) -> String {
    color.map_or_else(
        || "—".into(),
        |color| {
            if color & 255 == 255 {
                format!("#{:06X}", color >> 8)
            } else {
                format!("#{color:08X}")
            }
        },
    )
}

/// Render a localized empty state inside its owning viewport.
pub fn empty(title: &str, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .text_color(cx.theme().muted_foreground)
        .child(tr(title))
        .into_any_element()
}
/// Render a localized section heading.
pub fn section(title: &str) -> AnyElement {
    div()
        .px(rems(1.5))
        .py_4()
        .font_weight(FontWeight::SEMIBOLD)
        .child(tr(title))
        .into_any_element()
}
/// Format a service timestamp in UTC, retaining millisecond precision.
pub fn stamp(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|v| v.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// Decimal file sizes, matching macOS: 1 KB = 1,000 bytes.
pub fn file_size(bytes: u64) -> String {
    let units = [
        "ui.size_bytes",
        "ui.size_kb",
        "ui.size_mb",
        "ui.size_gb",
        "ui.size_tb",
        "ui.size_pb",
        "ui.size_eb",
    ];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000. && unit < units.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    // Promote a rounded boundary instead of displaying "1000 KB".
    value = (value * 10.).round() / 10.;
    if value >= 1000. && unit < units.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    let number = format!("{value:.1}");
    let number = number
        .strip_suffix(".0")
        .unwrap_or(&number)
        .replace('.', tr("ui.decimal_separator").as_ref());
    format!("{number}\u{a0}{}", tr(units[unit]))
}

/// Kit's plain Dialog requires an explicit footer; actions retain its focus trap.
pub fn dialog_actions() -> AnyElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            default_button("dialog-cancel")
                .label(tr("cancel"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(dialog::Cancel), cx)),
        )
        .child(
            Button::new("dialog-confirm")
                .primary()
                .label(tr("ui.confirm"))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(dialog::Confirm { secondary: false }), cx)
                }),
        )
        .into_any_element()
}
