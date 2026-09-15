use crate::{
    args::{Fields, Language},
    output::{CliError, message},
};
use std::{fs::File, io::Read, path::Path};
use taypeer_services::{EntryPatch, FieldUpdate};
use zeroize::Zeroizing;

const MAX_INPUT: u64 = 8 * 1024 * 1024;

pub(crate) struct Input {
    pub password_stdin: bool,
    pub language: Language,
}

impl Input {
    pub fn password(&self, creating: bool) -> Result<Zeroizing<String>, CliError> {
        if self.password_stdin {
            let bytes = read_bounded(std::io::stdin().lock())?;
            let value = std::str::from_utf8(&bytes).map_err(|_| CliError::Input)?;
            return Ok(Zeroizing::new(value.to_owned()));
        }
        let password = self.secret("master_password")?;
        if creating && *password != *self.secret("repeat_password")? {
            return Err(CliError::PasswordMismatch);
        }
        Ok(password)
    }

    pub(crate) fn secret(&self, key: &str) -> Result<Zeroizing<String>, CliError> {
        crate::secret_input::read(&message(self.language, key))
    }

    pub fn document<T: serde::de::DeserializeOwned>(&self, path: &Path) -> Result<T, CliError> {
        let bytes = if path == Path::new("-") {
            if self.password_stdin {
                return Err(CliError::StdinConflict);
            }
            read_bounded(std::io::stdin().lock())?
        } else {
            read_bounded(File::open(path).map_err(|_| CliError::Input)?)?
        };
        serde_json::from_slice(&bytes).map_err(|_| CliError::Input)
    }

    pub fn fields(&self, fields: Fields) -> Result<EntryPatch, CliError> {
        if let Some(path) = fields.input {
            return self.document(&path);
        }
        let update = |value: Option<String>| value.map_or(FieldUpdate::Keep, FieldUpdate::Set);
        let password = if fields.password_prompt {
            FieldUpdate::Set(self.secret("entry_password")?.to_string())
        } else if fields.clear_password {
            FieldUpdate::Clear
        } else {
            FieldUpdate::Keep
        };
        Ok(EntryPatch {
            title: update(fields.title),
            username: update(fields.username),
            url: update(fields.url),
            notes: update(fields.notes),
            password,
            ..Default::default()
        })
    }
}

fn read_bounded(reader: impl Read) -> Result<Zeroizing<Vec<u8>>, CliError> {
    let mut bytes = Zeroizing::new(Vec::new());
    reader
        .take(MAX_INPUT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| CliError::Input)?;
    if bytes.len() as u64 > MAX_INPUT {
        return Err(CliError::Input);
    }
    Ok(bytes)
}
