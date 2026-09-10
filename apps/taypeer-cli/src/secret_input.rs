//! Terminal input owned by the host while the command editor is suspended.

use crate::output::CliError;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    terminal::{disable_raw_mode, enable_raw_mode},
};
use std::io::Write;
use zeroize::{Zeroize, Zeroizing};

const MAX_SECRET_BYTES: usize = 8 * 1024 * 1024;

struct RawInput;
impl RawInput {
    fn enter() -> Result<Self, CliError> {
        enable_raw_mode().map_err(|_| CliError::Input)?;
        Ok(Self)
    }
    fn finish(self) -> Result<(), CliError> {
        // Drop retries restoration if the explicit attempt fails.
        disable_raw_mode().map_err(|_| CliError::Input)
    }
}
impl Drop for RawInput {
    fn drop(&mut self) {
        // Drop cannot report an error; the normal path calls finish explicitly.
        let _ = disable_raw_mode();
    }
}

pub(crate) fn read(prompt: &str) -> Result<Zeroizing<String>, CliError> {
    #[cfg(unix)]
    let terminal = "/dev/tty";
    #[cfg(windows)]
    let terminal = "CONOUT$";
    let mut output = std::fs::OpenOptions::new()
        .write(true)
        .open(terminal)
        .map_err(|_| CliError::Input)?;
    // Disable echo before publishing the prompt, including for pasted/PTY input.
    let raw = RawInput::enter()?;
    output
        .write_all(prompt.as_bytes())
        .map_err(|_| CliError::Input)?;
    output.flush().map_err(|_| CliError::Input)?;
    let result = read_events();
    let restored = raw.finish();
    writeln!(output).map_err(|_| CliError::Input)?;
    result.and_then(|secret| restored.map(|()| secret))
}

fn read_events() -> Result<Zeroizing<String>, CliError> {
    let mut value = Zeroizing::new(String::new());
    loop {
        let key = match event::read().map_err(|_| CliError::Input)? {
            Event::Key(key) => key,
            Event::Paste(mut text) => {
                let result = if text.chars().any(char::is_control) {
                    Err(CliError::Input)
                } else {
                    append(&mut value, &text)
                };
                text.zeroize();
                result?;
                continue;
            }
            _ => continue,
        };
        if key.kind == KeyEventKind::Release {
            continue;
        }
        match (key.code, key.modifiers) {
            (KeyCode::Enter, _) => return Ok(value),
            (KeyCode::Esc, _) | (KeyCode::Char('c' | 'd'), KeyModifiers::CONTROL) => {
                return Err(CliError::Input);
            }
            (KeyCode::Char('u'), KeyModifiers::CONTROL) => value = Zeroizing::new(String::new()),
            (KeyCode::Backspace, _) => {
                // Replacing the owner wipes the removed bytes too; String::pop alone
                // would leave them in the allocation beyond its new length.
                if let Some((end, _)) = value.char_indices().next_back() {
                    value = Zeroizing::new(value[..end].to_owned());
                }
            }
            (KeyCode::Char(character), modifiers)
                if !modifiers.intersects(KeyModifiers::CONTROL | KeyModifiers::ALT)
                    && !character.is_control() =>
            {
                let mut bytes = [0; 4];
                let result = append(&mut value, character.encode_utf8(&mut bytes));
                bytes.zeroize();
                result?;
            }
            _ => {}
        }
    }
}

fn append(value: &mut Zeroizing<String>, text: &str) -> Result<(), CliError> {
    let length = value.len().checked_add(text.len()).ok_or(CliError::Input)?;
    if length > MAX_SECRET_BYTES {
        return Err(CliError::Input);
    }
    if length > value.capacity() {
        // Grow through a fresh owner so the old allocation is erased before free.
        let capacity = length
            .max(value.capacity().saturating_mul(2))
            .clamp(64, MAX_SECRET_BYTES);
        let mut grown = Zeroizing::new(String::new());
        grown
            .try_reserve_exact(capacity)
            .map_err(|_| CliError::Input)?;
        grown.push_str(value);
        *value = grown;
    }
    value.push_str(text);
    Ok(())
}
