use crate::{
    args::{Action, Language, SessionLine},
    host::Host,
    output::{CliError, message, print_error, print_result},
};
use clap_repl::{ClapEditor, ReadCommandOutput};
use std::{
    io::{IsTerminal, Write},
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
        mpsc,
    },
    time::{Duration, Instant},
};

pub(crate) fn run(host: &mut Host, json: bool, language: Language) -> Result<(), CliError> {
    if !std::io::stdin().is_terminal() || host.input.password_stdin {
        return Err(CliError::Input);
    }
    writeln!(
        std::io::stderr().lock(),
        "{}",
        message(language, "session_ready")
    )
    .map_err(|_| CliError::Io)?;
    let (input_tx, input_rx) = mpsc::sync_channel(0);
    let (continue_tx, continue_rx) = mpsc::sync_channel(0);
    let activity = Arc::new(Activity {
        origin: Instant::now(),
        last: AtomicU64::new(0),
    });
    let editor_activity = Arc::clone(&activity);
    // Reedline restores terminal mode before yielding a command. Waiting for the
    // acknowledgement keeps it from competing with a hidden password prompt.
    std::thread::spawn(move || {
        use clap_repl::reedline::{
            Emacs, KeyCode, KeyModifiers, ReedlineEvent, default_emacs_keybindings,
        };
        let mut keys = default_emacs_keybindings();
        keys.add_binding(
            KeyModifiers::NONE,
            KeyCode::Tab,
            ReedlineEvent::UntilFound(vec![
                ReedlineEvent::Menu("completion_menu".into()),
                ReedlineEvent::MenuNext,
            ]),
        );
        let mut editor = ClapEditor::<SessionLine>::builder()
            .with_edit_mode(Box::new(ActiveEditMode {
                inner: Emacs::new(keys),
                activity: editor_activity,
            }))
            .build();
        loop {
            let event = editor.read_command();
            if input_tx.send(event).is_err() || continue_rx.recv().is_err() {
                break;
            }
        }
    });
    loop {
        match input_rx.recv_timeout(activity.remaining()) {
            Ok(ReadCommandOutput::Command(command)) => {
                let exit = matches!(command.action, Action::Exit);
                match host.execute(command.action) {
                    Ok(value) => print_result(value, json, language)?,
                    Err(error) => print_error(&error, json, language),
                }
                if exit {
                    return Ok(());
                }
            }
            Ok(ReadCommandOutput::CtrlD) => return Ok(()),
            Ok(ReadCommandOutput::CtrlC | ReadCommandOutput::EmptyLine) => {}
            Ok(ReadCommandOutput::ClapError(error)) => {
                error.print().map_err(|_| CliError::Io)?;
            }
            Ok(ReadCommandOutput::ShlexError) => print_error(&CliError::Input, json, language),
            Ok(ReadCommandOutput::ReedlineError(_)) => return Err(CliError::Io),
            Err(mpsc::RecvTimeoutError::Timeout) => {
                if !activity.remaining().is_zero() {
                    continue;
                }
                if let Err(error) = host.lock_all() {
                    print_error(&error, json, language);
                }
                writeln!(
                    std::io::stderr().lock(),
                    "{}",
                    message(language, "session_locked")
                )
                .map_err(|_| CliError::Io)?;
                activity.touch();
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CliError::Io),
        }
        activity.touch();
        if continue_tx.send(()).is_err() {
            return Err(CliError::Io);
        }
    }
}

// The editor owns terminal input; the host owns workers. Only an activity timestamp
// crosses threads, so a long unfinished command does not count as inactivity.
struct Activity {
    origin: Instant,
    last: AtomicU64,
}
impl Activity {
    fn touch(&self) {
        self.last
            .store(self.origin.elapsed().as_millis() as u64, Ordering::Relaxed);
    }
    fn remaining(&self) -> Duration {
        let elapsed = (self.origin.elapsed().as_millis() as u64)
            .saturating_sub(self.last.load(Ordering::Relaxed));
        Duration::from_millis(300_000u64.saturating_sub(elapsed))
    }
}
struct ActiveEditMode {
    inner: clap_repl::reedline::Emacs,
    activity: Arc<Activity>,
}
impl clap_repl::reedline::EditMode for ActiveEditMode {
    fn parse_event(
        &mut self,
        event: clap_repl::reedline::ReedlineRawEvent,
    ) -> clap_repl::reedline::ReedlineEvent {
        self.activity.touch();
        self.inner.parse_event(event)
    }
    fn edit_mode(&self) -> clap_repl::reedline::PromptEditMode {
        self.inner.edit_mode()
    }
}
