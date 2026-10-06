//! Main-thread component renderer; deliberately independent of AppView and workers.
use serde::Deserialize;
use std::{collections::HashSet, path::PathBuf};

#[cfg(target_os = "macos")]
mod macos;

type Result<T> = std::result::Result<T, Box<dyn std::error::Error + Send + Sync>>;
const CATALOG: &str = include_str!("cases.json");

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Catalog {
    schema_version: u8,
    cases: Vec<Case>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    id: String,
    state: State,
    kind: Kind,
    label: String,
    text: String,
    width: u32,
    height: u32,
    design: Design,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "kebab-case")]
enum Kind {
    Field,
    RowField,
    EditorRow,
}

#[derive(Clone, Copy, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
enum State {
    Rest,
    Focus,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Design {
    page: String,
    node: String,
    #[serde(default)]
    ancestors: Vec<String>,
    #[serde(default)]
    anchors: std::collections::BTreeMap<String, String>,
}

struct Options {
    output: Option<PathBuf>,
    cases: Vec<String>,
    dark: bool,
    font_size: u8,
    list: bool,
}

impl Options {
    fn parse() -> Result<Self> {
        let mut options = Self {
            output: None,
            cases: Vec::new(),
            dark: true,
            font_size: 16,
            list: false,
        };
        let mut args = std::env::args().skip(1);
        while let Some(arg) = args.next() {
            match arg.as_str() {
                "--list" => options.list = true,
                "--output" => {
                    options.output = Some(args.next().ok_or("missing --output value")?.into())
                }
                "--case" => options
                    .cases
                    .push(args.next().ok_or("missing --case value")?),
                "--theme" => {
                    options.dark = match args.next().as_deref() {
                        Some("dark") => true,
                        Some("light") => false,
                        _ => return Err("--theme must be dark or light".into()),
                    };
                }
                "--font-size" => {
                    options.font_size = args.next().ok_or("missing --font-size value")?.parse()?;
                    if ![14, 16, 18].contains(&options.font_size) {
                        return Err("--font-size must be 14, 16 or 18".into());
                    }
                }
                _ => return Err(format!("unknown component renderer argument: {arg}").into()),
            }
        }
        Ok(options)
    }
}

fn catalog() -> Result<Catalog> {
    let catalog: Catalog = serde_json::from_str(CATALOG)?;
    if catalog.schema_version != 2 || catalog.cases.is_empty() {
        return Err("unsupported or empty component catalog".into());
    }
    let mut ids = HashSet::new();
    for case in &catalog.cases {
        if case.id.is_empty()
            || !case
                .id
                .bytes()
                .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == b'-')
            || !ids.insert(&case.id)
            || !(1..=4096).contains(&case.width)
            || !(1..=4096).contains(&case.height)
            || case.text.contains(['\n', '\0'])
            || case.design.page.is_empty()
            || case.design.node.is_empty()
            || !["name", "url"].contains(&case.label.as_str())
            || case.design.ancestors.iter().any(String::is_empty)
            || (case.kind == Kind::EditorRow
                && (case.design.ancestors.is_empty()
                    || ["label", "field", "text"]
                        .iter()
                        .any(|role| case.design.anchors.get(*role).is_none_or(String::is_empty))))
            || (case.kind != Kind::EditorRow
                && !case.design.node.ends_with(match case.state {
                    State::Rest => "/ rest",
                    State::Focus => "/ focus",
                }))
        {
            return Err("invalid component identity, dimensions or design target".into());
        }
    }
    Ok(catalog)
}

fn run() -> Result<()> {
    let options = Options::parse()?;
    let catalog = catalog()?;
    if options.list {
        println!("{CATALOG}");
        return Ok(());
    }
    if options
        .cases
        .iter()
        .any(|id| !catalog.cases.iter().any(|case| &case.id == id))
    {
        return Err("unknown component case; use --list".into());
    }
    let cases: Vec<_> = catalog
        .cases
        .iter()
        .filter(|case| options.cases.is_empty() || options.cases.contains(&case.id))
        .collect();
    if cases.is_empty() {
        return Err("no matching component case; use --list".into());
    }
    #[cfg(target_os = "macos")]
    return macos::run(&cases, &options);
    #[cfg(not(target_os = "macos"))]
    {
        // Cargo's workspace suite remains portable; an explicit capture must fail.
        if options.output.is_some() || !options.cases.is_empty() {
            return Err("component screenshots require the macOS Metal headless renderer".into());
        }
        println!(
            "component screenshots skipped: macOS Metal required ({} cases; {} theme, {}px)",
            cases.len(),
            if options.dark { "dark" } else { "light" },
            options.font_size
        );
        Ok(())
    }
}

fn main() {
    if let Err(error) = run() {
        eprintln!("component renderer: {error}");
        std::process::exit(1);
    }
}
