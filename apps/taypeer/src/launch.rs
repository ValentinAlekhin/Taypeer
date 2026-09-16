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
            [flag, path] if flag == "--profile" && !path.is_empty() => Ok(Self::Ui),
            [arg] if arg == "--smoke-test" => Ok(Self::Smoke),
            _ => Err("Usage: taypeer [--profile PATH] | --smoke-test"),
        }
    }
}
/// Public profile override is an explicit development/test launch option, never a secret.
pub(crate) fn profile(args: &[String]) -> Result<Option<std::path::PathBuf>, &'static str> {
    match args {
        [] => Ok(None),
        [flag, path] if flag == "--profile" && !path.is_empty() => Ok(Some(path.into())),
        _ => Err("Usage: taypeer [--profile PATH] | --smoke-test"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn explicit_profile_is_ui_only_and_requires_a_path() {
        let args = ["--profile".into(), "/tmp/PUBLIC-test-profile".into()];
        assert_eq!(LaunchMode::parse(&args), Ok(LaunchMode::Ui));
        assert_eq!(
            profile(&args).unwrap(),
            Some("/tmp/PUBLIC-test-profile".into())
        );
        assert!(LaunchMode::parse(&["--profile".into()]).is_err());
        assert!(LaunchMode::parse(&["--profile".into(), "".into()]).is_err());
        assert!(
            LaunchMode::parse(&["--profile".into(), "PUBLIC".into(), "--smoke-test".into()])
                .is_err()
        );
    }
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
