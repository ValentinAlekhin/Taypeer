//! Command-line fixture consumes a PUBLIC SPKI and creates a new PUBLIC ciphertext file.
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};
use taypeer_public_biometric_wrap::{SPKI_LIMIT, WrapError, wrap_public_fixture};

enum CommandError {
    Usage,
    Input,
    Wrap(WrapError),
    Output,
}
impl CommandError {
    fn category(&self) -> &'static str {
        match self {
            Self::Usage => "expected PUBLIC SPKI input path and new PUBLIC ciphertext output path",
            Self::Input => "PUBLIC input could not be read",
            Self::Wrap(WrapError::InputLength) => "PUBLIC SPKI length is outside the bound",
            Self::Wrap(WrapError::Spki) => "PUBLIC input is not exact RSA DER SPKI",
            Self::Wrap(WrapError::Parameters) => "PUBLIC RSA parameters must be 2048 bits and F4",
            Self::Wrap(WrapError::Encryption) => "PUBLIC OAEP encryption failed",
            Self::Output => "new PUBLIC output could not be created or completed",
        }
    }
}

fn wrap_file(input: &Path, output: &Path) -> Result<(), CommandError> {
    let mut bytes = Vec::new();
    File::open(input)
        .map_err(|_| CommandError::Input)?
        .take(SPKI_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CommandError::Input)?;
    let ciphertext = wrap_public_fixture(&bytes).map_err(CommandError::Wrap)?;
    // Do not replace an existing fixture or mistake output publication for biometric enablement.
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output)
        .map_err(|_| CommandError::Output)?;
    file.write_all(&ciphertext)
        .map_err(|_| CommandError::Output)?;
    file.sync_all().map_err(|_| CommandError::Output)
}

fn run() -> Result<(), CommandError> {
    let mut arguments = std::env::args_os().skip(1);
    let input = arguments.next().ok_or(CommandError::Usage)?;
    let output = arguments.next().ok_or(CommandError::Usage)?;
    if arguments.next().is_some() {
        return Err(CommandError::Usage);
    }
    wrap_file(Path::new(&input), Path::new(&output))
}

fn main() {
    if let Err(error) = run() {
        eprintln!("{}", error.category());
        std::process::exit(2);
    }
}
