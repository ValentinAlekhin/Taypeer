//! Explicit, mutually exclusive application entry points.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchMode {
    Ui,
    Smoke,
}
impl LaunchMode {
    pub fn parse(args: &[String]) -> Result<Self, &'static str> {
        match args {
            [] => Ok(Self::Ui),
            [arg] if arg == "--smoke-test" => Ok(Self::Smoke),
            _ => Err("Usage: taypeer [--smoke-test]"),
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn only_ui_and_headless_smoke_are_supported() {
        assert_eq!(LaunchMode::parse(&[]), Ok(LaunchMode::Ui));
        assert_eq!(
            LaunchMode::parse(&["--smoke-test".into()]),
            Ok(LaunchMode::Smoke)
        );
        for flag in ["--legacy-ui", "--demo", "--unknown"] {
            assert!(LaunchMode::parse(&[flag.into()]).is_err());
        }
        assert!(LaunchMode::parse(&["--smoke-test".into(), "--smoke-test".into()]).is_err());
    }
}
