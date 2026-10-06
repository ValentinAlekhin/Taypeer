use super::{Case, Options, Result, State};
use gpui_kit::{
    component::{ActiveTheme, Root, input::InputState},
    prelude::*,
    test::TestWindowExt,
    *,
};
use serde::Serialize;
use std::sync::Arc;
use taypeer_ui::{assets::ProductAssets, fonts, style, theme};

// A fixed logical margin preserves focus paint outside the component's box.
const CAPTURE_PADDING: u8 = 4;

struct FieldPreview {
    input: Entity<InputState>,
}

impl Render for FieldPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .size_full()
            .p(px(f32::from(CAPTURE_PADDING)))
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(style::field(&self.input, "name").w_full())
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
    let height = case.height as f32 * factor + f32::from(CAPTURE_PADDING) * 2.;
    let window = cx.open_window(size(px(width), px(height)), |window, cx| {
        theme::apply(dark, font_size, window, cx);
        // Headless TestWindow activation only delivers a simulated lifecycle event.
        // BaseInput hides the caret in an inactive window, even with field focus.
        window.activate_window();
        let input = style::input(&case.text, false, window, cx);
        let preview = cx.new(|_| FieldPreview { input });
        cx.new(|cx| Root::new(preview, window, cx))
    })?;
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        if case.state == State::Focus {
            // Focus after mounting through the real mouse handler, so the input's
            // retained listeners and cursor lifecycle observe the transition.
            window.click("field-name", cx);
        }
    })?;
    // The test executor keeps virtual time fixed: the focus caret cannot blink
    // between captures. No cursor masking or modifications to product paint.
    cx.run_until_parked();
    cx.update_window(window.into(), |_, window, cx| {
        window.render_frame(cx);
        let field = window.find("field-name");
        assert!(window.is_window_active(), "{}: active test window", case.id);
        assert!(field.visible(), "{}: input must be visible", case.id);
        assert_eq!(
            field.focused(),
            Some(case.state == State::Focus),
            "{}: focus",
            case.id
        );
        assert_eq!(cx.theme().font_family.as_ref(), "Inter");
    })?;
    let image = cx.capture_screenshot(window.into())?;
    assert_eq!(
        image.dimensions(),
        ((width * 2.) as u32, (height * 2.) as u32)
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
