use crate::{
    args::{Action, Language, SessionLine},
    host::Host,
    output::{CliError, message, print_error, print_result},
};
use clap_repl::{ClapEditor, ReadCommandOutput};
use std::{
    io::{IsTerminal, Write},
    sync::mpsc,
    time::Duration,
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
    let activity = host.sessions.activity();
    let editor_activity = activity.clone();
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
        for outcome in host.collect_closed()? {
            print_result(
                serde_json::json!({"event": "session_locked", "outcome": outcome}),
                json,
                language,
            )?;
        }
        match input_rx.recv_timeout(Duration::from_millis(100)) {
            Ok(ReadCommandOutput::Command(command)) => {
                let exit = matches!(command.action, Action::Exit);
                activity.touch();
                let epoch = activity.epoch();
                match host.execute(command.action) {
                    Ok(value) => {
                        if let Err(error) = crate::output::print_checked_result(
                            value, json, language, &activity, epoch,
                        ) {
                            print_error(&error, json, language);
                        }
                    }
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
            Err(mpsc::RecvTimeoutError::Timeout) => continue,
            Err(mpsc::RecvTimeoutError::Disconnected) => return Err(CliError::Io),
        }
        if continue_tx.send(()).is_err() {
            return Err(CliError::Io);
        }
    }
}

struct ActiveEditMode {
    inner: clap_repl::reedline::Emacs,
    activity: taypeer_runtime::session::ActivityHandle,
}
impl clap_repl::reedline::EditMode for ActiveEditMode {
    fn parse_event(
        &mut self,
        event: clap_repl::reedline::ReedlineRawEvent,
    ) -> clap_repl::reedline::ReedlineEvent {
        let event: crossterm::event::Event = event.into();
        if matches!(
            event,
            crossterm::event::Event::Key(_) | crossterm::event::Event::Paste(_)
        ) {
            self.activity.touch();
        }
        self.inner.parse_event(
            event
                .try_into()
                .expect("ReedlineRawEvent already normalized this event"),
        )
    }
    fn edit_mode(&self) -> clap_repl::reedline::PromptEditMode {
        self.inner.edit_mode()
    }
}
