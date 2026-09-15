use crate::args::Language;
use serde_json::Value;
use std::{collections::BTreeMap, io::Write, sync::OnceLock};
use taypeer_runtime::RuntimeError;
use zeroize::Zeroizing;

#[derive(Debug)]
pub(crate) enum CliError {
    Runtime(RuntimeError),
    Input,
    PasswordMismatch,
    StdinConflict,
    NoDatabase,
    UnknownDatabase,
    AlreadyOpen,
    SessionOnly,
    Io,
}
impl From<RuntimeError> for CliError {
    fn from(error: RuntimeError) -> Self {
        Self::Runtime(error)
    }
}
impl CliError {
    pub fn key(&self) -> &'static str {
        match self {
            Self::Runtime(RuntimeError::SessionClosed(_)) => "session_closed",
            Self::Runtime(RuntimeError::OperationInterrupted(_)) => "operation_interrupted",
            Self::Runtime(RuntimeError::ShutdownUnconfirmed) => "shutdown_unconfirmed",
            Self::Runtime(RuntimeError::Profile(taypeer_runtime::profile::ProfileError::Busy)) => {
                "profile_busy"
            }
            Self::Runtime(
                RuntimeError::Profile(taypeer_runtime::profile::ProfileError::Credentials)
                | RuntimeError::Service(taypeer_services::ServiceError::Credentials),
            ) => "credentials_error",
            Self::Runtime(RuntimeError::Service(taypeer_services::ServiceError::ReadOnly)) => {
                "read_only"
            }
            Self::Runtime(RuntimeError::Service(
                taypeer_services::ServiceError::ReadCompatibility,
            )) => "compatibility_read",
            Self::Runtime(RuntimeError::Service(
                taypeer_services::ServiceError::WriteCompatibility,
            )) => "compatibility_write",
            Self::Runtime(RuntimeError::Service(
                taypeer_services::ServiceError::Storage(
                    taypeer_services::StorageError::UnsupportedVersion
                    | taypeer_services::StorageError::Trust(taypeer_trust::Error::UnsupportedVersion),
                )
                | taypeer_services::ServiceError::Trust(taypeer_trust::Error::UnsupportedVersion),
            )) => "compatibility_encoding",
            Self::Runtime(RuntimeError::Service(taypeer_services::ServiceError::AwaitingData)) => {
                "awaiting_data"
            }
            Self::Runtime(RuntimeError::Service(
                taypeer_services::ServiceError::Trust(_)
                | taypeer_services::ServiceError::Unauthorized,
            )) => "authority_error",
            Self::Runtime(RuntimeError::Service(
                taypeer_services::ServiceError::Storage(
                    taypeer_services::StorageError::Changed
                    | taypeer_services::StorageError::CommitUncertain,
                )
                | taypeer_services::ServiceError::ExpiredSession,
            )) => "stale_generation",
            Self::Runtime(taypeer_runtime::RuntimeError::Service(
                taypeer_services::ServiceError::AttachmentLimit,
            )) => "attachment_limit",
            Self::Runtime(taypeer_runtime::RuntimeError::Service(
                taypeer_services::ServiceError::Icon(
                    taypeer_services::icons::IconError::NetworkBoundary,
                ),
            )) => "icon_network_boundary",
            Self::Runtime(taypeer_runtime::RuntimeError::Service(
                taypeer_services::ServiceError::Icon(_),
            )) => "icon_error",
            Self::Runtime(taypeer_runtime::RuntimeError::Service(
                taypeer_services::ServiceError::Storage(
                    taypeer_services::StorageError::MissingBlob,
                ),
            )) => "missing_blob",
            Self::Runtime(_) => "runtime_error",
            Self::Input => "input_error",
            Self::PasswordMismatch => "password_mismatch",
            Self::StdinConflict => "stdin_conflict",
            Self::NoDatabase => "no_database",
            Self::UnknownDatabase => "unknown_database",
            Self::AlreadyOpen => "already_open",
            Self::SessionOnly => "session_only",
            Self::Io => "io_error",
        }
    }
}

fn catalog(language: Language) -> &'static BTreeMap<String, String> {
    type Catalog = BTreeMap<String, String>;
    static EN: OnceLock<Catalog> = OnceLock::new();
    static RU: OnceLock<Catalog> = OnceLock::new();
    let (cell, source) = match language {
        Language::En => (&EN, include_str!("../locales/en.json")),
        Language::Ru => (&RU, include_str!("../locales/ru.json")),
    };
    cell.get_or_init(|| {
        serde_json::from_str(source).expect("embedded CLI translations are validated by tests")
    })
}

pub(crate) fn message(language: Language, key: &str) -> String {
    catalog(language)
        .get(key)
        .cloned()
        .unwrap_or_else(|| key.into())
}

/// Clap builds help before parsing its language flag, including in the session editor.
pub(crate) fn help(key: &'static str) -> &'static str {
    static LANGUAGE: OnceLock<Language> = OnceLock::new();
    let language = LANGUAGE.get_or_init(|| {
        let mut args = std::env::args_os();
        while let Some(arg) = args.next() {
            if arg == "--lang=ru"
                || (arg == "--lang" && args.next().is_some_and(|value| value == "ru"))
            {
                return Language::Ru;
            }
        }
        Language::En
    });
    catalog(*language)
        .get(key)
        .map(String::as_str)
        .unwrap_or(key)
}

pub(crate) fn print_result(
    mut value: Value,
    json: bool,
    language: Language,
) -> Result<(), CliError> {
    let result = write_result(&value, json, language);
    taypeer_runtime::erase_view(&mut value);
    result
}

pub(crate) fn print_checked_result(
    mut value: Value,
    json: bool,
    language: Language,
    activity: &taypeer_runtime::session::ActivityHandle,
    epoch: u64,
) -> Result<(), CliError> {
    if activity.epoch() != epoch {
        taypeer_runtime::erase_view(&mut value);
        return Err(RuntimeError::OperationInterrupted(
            activity
                .reason()
                .unwrap_or(taypeer_runtime::session::LockReason::HostExited),
        )
        .into());
    }
    print_result(value, json, language)
}

fn write_result(value: &Value, json: bool, language: Language) -> Result<(), CliError> {
    let text = if json {
        serde_json::to_string(value).map_err(|_| CliError::Io)?
    } else {
        match value {
            Value::Null => message(language, "done"),
            Value::String(value) => value.clone(),
            _ => serde_json::to_string_pretty(value).map_err(|_| CliError::Io)?,
        }
    };
    let text = Zeroizing::new(text);
    writeln!(std::io::stdout().lock(), "{}", text.as_str()).map_err(|_| CliError::Io)
}

pub(crate) fn print_error(error: &CliError, json: bool, language: Language) {
    let detail = match error {
        CliError::Runtime(error) => Some(error),
        _ => None,
    };
    let output = if json {
        serde_json::json!({"error": {"code": error.key(), "message": message(language, error.key()), "detail": detail}}).to_string()
    } else {
        match detail {
            Some(detail) => format!("{} ({detail})", message(language, error.key())),
            None => message(language, error.key()),
        }
    };
    // Reporting to a closed stderr cannot repair the original operation.
    let _ = writeln!(std::io::stderr().lock(), "{output}");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_late_foreground_result_is_rejected_after_invalidation() {
        use taypeer_runtime::session::{LockReason, SessionController, SessionPolicy};
        let sessions = SessionController::new(SessionPolicy::default());
        let input = sessions.activity();
        let epoch = input.epoch();
        sessions.lock_all(LockReason::SystemLocked);
        let result = print_checked_result(
            Value::String("PUBLIC late result".into()),
            true,
            Language::En,
            &input,
            epoch,
        );
        assert!(matches!(
            result,
            Err(CliError::Runtime(RuntimeError::OperationInterrupted(
                LockReason::SystemLocked
            )))
        ));
    }
    #[test]
    fn unsupported_signed_encoding_is_distinct_from_damaged_authority() {
        let error = CliError::from(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_services::StorageError::Trust(
                taypeer_trust::Error::UnsupportedVersion,
            )),
        ));
        assert_eq!(error.key(), "compatibility_encoding");
    }

    #[test]
    fn catalogs_have_identical_keys_and_no_empty_messages() {
        let en: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/en.json")).unwrap();
        let ru: BTreeMap<String, String> =
            serde_json::from_str(include_str!("../locales/ru.json")).unwrap();
        assert_eq!(en.keys().collect::<Vec<_>>(), ru.keys().collect::<Vec<_>>());
        assert!(en.values().chain(ru.values()).all(|s| !s.is_empty()));
    }
}
