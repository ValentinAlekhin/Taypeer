use super::{Case, Kind, Options, Result, State};
use gpui_kit::{
    component::{ActiveTheme, Root, input::InputState},
    prelude::*,
    test::TestWindowExt,
    *,
};
use serde::Serialize;
use std::{collections::BTreeMap, sync::Arc};
use taypeer_ui::{assets::ProductAssets, clipboard, fonts, style, theme};

// A fixed logical margin preserves focus paint outside the component's box.
const CAPTURE_PADDING: u8 = 4;

struct FieldPreview {
    input: Entity<InputState>,
    kind: Kind,
    label: String,
    width: f32,
}

impl Render for FieldPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let component = if self.kind == Kind::EditorRow {
            let mut control = style::input_control(
                &self.label,
                clipboard::secret_field(
                    style::row_field(&self.input, &self.label),
                    &self.input,
                    false,
                ),
            );
            if self.label == "url" {
                control = control
                    .child(style::icon_button("favicon", "download", "ui.favicon"))
                    .child(style::icon_button(
                        "clear-field-preview",
                        "x",
                        "ui.clear_value",
                    ));
            }
            style::input_row(&self.label, self.input.focus_handle(cx), control, cx)
        } else {
            let field = if self.kind == Kind::RowField {
                style::row_field(&self.input, &self.label)
            } else {
                style::field(&self.input, &self.label)
            };
            field.into_any_element()
        };
        div()
            .p(px(f32::from(CAPTURE_PADDING)))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(
                div()
                    .id("component")
                    .test_support()
                    .w(px(self.width))
                    .child(component),
            )
    }
}

#[derive(Clone, Copy, Serialize)]
struct Rect {
    x: f32,
    y: f32,
    width: f32,
    height: f32,
}
impl Rect {
    fn from_bounds(bounds: Bounds<Pixels>) -> Self {
        Self {
            x: f32::from(bounds.origin.x) - f32::from(CAPTURE_PADDING),
            y: f32::from(bounds.origin.y) - f32::from(CAPTURE_PADDING),
            width: f32::from(bounds.size.width),
            height: f32::from(bounds.size.height),
        }
    }
}

#[derive(Serialize)]
struct Capture<'a> {
    id: &'a str,
    file: String,
    width: u32,
    height: u32,
    scale: u8,
    padding: u8,
    theme: &'static str,
    font_size: u8,
    anchors: BTreeMap<String, Rect>,
}

fn measure_layout(
    case: &Case,
    input: &Entity<InputState>,
    window: &Window,
    cx: &App,
) -> BTreeMap<String, Rect> {
    let field = window.find(format!("field-{}", case.label));
    assert!(window.is_window_active(), "{}: active test window", case.id);
    assert!(field.visible(), "{}: input must be visible", case.id);
    assert_eq!(
        field.focused(),
        Some(case.state == State::Focus),
        "{}: focus",
        case.id
    );
    assert_eq!(cx.theme().font_family.as_ref(), "Inter");
    let component_id = if case.kind == Kind::EditorRow {
        format!("row-{}", case.label)
    } else {
        "component".to_owned()
    };
    let mut anchors = BTreeMap::from([
        (
            "component".into(),
            Rect::from_bounds(window.find(component_id).bounds()),
        ),
        ("field".into(), Rect::from_bounds(field.bounds())),
    ]);
    let input = input.read(cx);
    anchors.insert("text".into(), Rect::from_bounds(input.input_bounds()));
    if case.kind == Kind::EditorRow {
        anchors.insert(
            "label".into(),
            Rect::from_bounds(window.find(format!("label-{}", case.label)).bounds()),
        );
        if case.label == "url" {
            for (role, id) in [
                ("action.favicon", "favicon"),
                ("action.clear", "clear-field-preview"),
            ] {
                anchors.insert(role.into(), Rect::from_bounds(window.find(id).bounds()));
            }
        }
    }
    anchors
}

fn render<'a>(
    case: &'a Case,
    dark: bool,
    font_size: u8,
    output: Option<&std::path::Path>,
) -> Result<Capture<'a>> {
    let mut cx = HeadlessAppContext::with_platform(
        gpui_kit::platform::current_platform(true).text_system(),
        Arc::new(ProductAssets),
        gpui_kit::platform::current_headless_renderer,
    );
    cx.update(|cx| -> gpui_kit::Result<()> {
        fonts::register(cx)?;
        gpui_kit::init(cx);
        Ok(())
    })?;
    let factor = f32::from(font_size) / 16.;
    let width = case.width as f32 * factor + f32::from(CAPTURE_PADDING) * 2.;
    // Height is not constrained to the mockup. Measure the retained layout in a
    // roomy viewport, then crop only unused viewport space, preserving overflow.
    let height = 1024.;
    let mut retained_input = None;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        theme::apply(dark, font_size, window, cx);
        // Headless TestWindow activation only delivers a simulated lifecycle event.
        // BaseInput hides the caret in an inactive window, even with field focus.
        window.activate_window();
        let input = style::input(&case.text, false, window, cx);
        retained_input = Some(input.clone());
        let preview = cx.new(|_| FieldPreview {
            input,
            kind: case.kind,
            label: case.label.clone(),
            width: case.width as f32 * factor,
        });
        cx.new(|cx| Root::new(preview, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        if case.state == State::Focus {
            // Focus after mounting through the real mouse handler, so the input's
            // retained listeners and cursor lifecycle observe the transition.
            window.click(format!("field-{}", case.label), cx);
        }
    })?;
    // The test executor keeps virtual time fixed: the focus caret cannot blink
    // between captures. No cursor masking or modifications to product paint.
    cx.run_until_parked();
    let anchors = cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        measure_layout(
            case,
            retained_input.as_ref().expect("preview input mounted"),
            window,
            cx,
        )
    })?;
    let image = cx.capture_screenshot(window.into())?;
    // Metal rounds fractional logical extents up to whole device pixels.
    assert_eq!(
        image.dimensions(),
        ((width * 2.).ceil() as u32, (height * 2.).ceil() as u32)
    );
    assert!(
        image.pixels().any(|pixel| pixel != image.get_pixel(0, 0)),
        "{}: nonempty paint",
        case.id
    );
    cx.update_window(window.into(), |_, window, cx| window.render_frame(cx))?;
    assert_eq!(
        image,
        cx.capture_screenshot(window.into())?,
        "{}: repeatable pixels",
        case.id
    );
    let right = anchors.values().map(|r| r.x + r.width).fold(0., f32::max);
    let bottom = anchors.values().map(|r| r.y + r.height).fold(0., f32::max);
    if anchors.values().any(|r| {
        r.x < -f32::from(CAPTURE_PADDING)
            || r.y < -f32::from(CAPTURE_PADDING)
            || ![r.x, r.y, r.width, r.height].iter().all(|v| v.is_finite())
            || r.width <= 0.
            || r.height <= 0.
    }) {
        return Err(format!(
            "{}: invalid layout bounds or overflow beyond capture margin",
            case.id
        )
        .into());
    }
    let capture_width = ((right + f32::from(CAPTURE_PADDING) * 2.) * 2.).ceil() as u32;
    let capture_height = ((bottom + f32::from(CAPTURE_PADDING) * 2.) * 2.).ceil() as u32;
    if capture_width > image.width() || capture_height > image.height() {
        return Err(format!("{}: layout exceeds capture viewport", case.id).into());
    }
    let image = image::imageops::crop_imm(&image, 0, 0, capture_width, capture_height).to_image();
    let file = format!("{}.png", case.id);
    if let Some(output) = output {
        image.save(output.join(&file))?;
    }
    Ok(Capture {
        id: &case.id,
        file,
        width: image.width(),
        height: image.height(),
        scale: 2,
        padding: CAPTURE_PADDING,
        theme: if dark { "dark" } else { "light" },
        font_size,
        anchors,
    })
}

pub(super) fn run(cases: &[&Case], options: &Options) -> Result<()> {
    rust_i18n::set_locale("ru");
    if let Some(output) = &options.output {
        std::fs::create_dir_all(output)?;
        let captures = cases
            .iter()
            .map(|case| render(case, options.dark, options.font_size, Some(output)))
            .collect::<Result<Vec<_>>>()?;
        std::fs::write(
            output.join("captures.json"),
            serde_json::to_vec_pretty(&captures)?,
        )?;
        println!(
            "Rendered {} isolated components into {}",
            captures.len(),
            output.display()
        );
    } else {
        for dark in [false, true] {
            for font_size in [14, 16, 18] {
                for case in cases {
                    render(case, dark, font_size, None)?;
                }
            }
        }
        println!(
            "Component rendering: {} cases × 2 themes × 3 font sizes passed",
            cases.len()
        );
    }
    Ok(())
}
