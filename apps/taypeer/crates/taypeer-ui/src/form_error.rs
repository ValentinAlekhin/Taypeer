//! Localized validation errors shared by product forms.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
/// Safe, localized validation outcomes shared by feature forms.
pub enum FormError {
    /// A required name is empty or invalid.
    RequiredName,
    /// An attribute with this name already exists.
    DuplicateAttribute,
    /// The edited domain object no longer exists.
    MissingObject,
    /// Password confirmation does not match.
    PasswordConfirmation,
    /// A numeric value is outside the accepted domain.
    InvalidNumber,
    /// The operation failed without a more specific public error.
    Backend,
    /// The user canceled before commitment.
    Canceled,
    /// A runtime failure safe to translate through the client error map.
    Runtime(taypeer_runtime::RuntimeError),
}
impl FormError {
    /// Return the resource key without exposing submitted values.
    pub fn key(self) -> &'static str {
        match self {
            Self::RequiredName => "ui.required_name",
            Self::DuplicateAttribute => "ui.duplicate_attribute",
            Self::MissingObject => "ui.missing_object",
            Self::PasswordConfirmation => "ui.password_confirmation",
            Self::InvalidNumber => "ui.invalid_number",
            Self::Backend => "ui.operation_failed",
            Self::Canceled => "close",
            Self::Runtime(error) => taypeer_runtime_client::error_key(&error),
        }
    }
}
/// Validate a product name using the shared domain rule.
pub fn require_name(name: &str) -> Result<(), FormError> {
    taypeer_core::validate_group_name(name).map_err(|_| FormError::RequiredName)
}
