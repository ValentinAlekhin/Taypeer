//! Owns native file selection and one serialized background service operation.

use super::{Client, Form, Navigation};
use crate::macos::common::{input, tr};
use gpui_kit::*;
use std::path::PathBuf;
use taypeer_services::{DatabaseService, ServiceError, SessionToken};
use zeroize::Zeroizing;

impl Client {
    pub(super) fn run_io<T: Send + 'static>(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
        operation: impl FnOnce(&mut DatabaseService) -> Result<T, ServiceError> + Send + 'static,
        complete: impl FnOnce(&mut Self, Result<T, ServiceError>, &mut Window, &mut Context<Self>)
        + 'static,
    ) {
        if self.busy {
            return;
        }
        self.busy = true;
        self.revealed.clear();
        self.root_focus.focus(window, cx);
        let mut service = std::mem::take(&mut self.service);
        let work = cx.background_executor().spawn(async move {
            let result = operation(&mut service);
            (service, result)
        });
        self.io_task = Some(cx.spawn_in(window, async move |client, cx| {
            let (service, result) = work.await;
            // A dropped window discards the response and its owned service, never resurrecting UI.
            let _ = client.update_in(cx, move |this, window, cx| {
                this.service = service;
                this.busy = false;
                if this.lock_requested {
                    this.lock_requested = false;
                    this.session = None;
                    this.sessions.clear();
                    this.clear_content(window, cx);
                    this.lock_after_io(window, cx);
                    return;
                }
                complete(this, result, window, cx);
                cx.notify();
            });
        }));
        cx.notify();
    }

    fn lock_after_io(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.run_io(
            window,
            cx,
            |service| service.lock_all_checked(),
            |this, result, _, cx| {
                this.error = result.err().map(|_| "file_draft_error");
                cx.notify();
            },
        );
    }

    pub(super) fn accept_file_session(
        &mut self,
        result: Result<SessionToken, ServiceError>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match result {
            Ok(token) => {
                self.clear_content(window, cx);
                self.database = Some(token.database.clone());
                self.sessions.insert(token.database.clone(), token.clone());
                self.restore = self
                    .service
                    .pending_draft(&token)
                    .is_ok_and(|reply| reply.value.is_some());
                self.session = Some(token);
                self.error = None;
                if self.restore {
                    self.modal_focus.focus(window, cx);
                }
            }
            Err(error) => {
                self.error = Some(file_error(error));
                self.bind_prompt_inputs(window, cx);
                let field = if self.form.is_some() {
                    &self.file_password
                } else {
                    &self.password
                };
                field.update(cx, |state, cx| state.focus(window, cx));
            }
        }
    }

    pub(super) fn choose_file(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let paths = cx.prompt_for_paths(PathPromptOptions {
            files: true,
            directories: false,
            multiple: false,
            prompt: Some(tr("open_db")),
        });
        self.io_task = Some(cx.spawn_in(window, async move |client, cx| {
            match paths.await {
                Ok(Ok(Some(paths))) => {
                    if let Some(path) = paths.into_iter().next() {
                        let _ = client.update_in(cx, |this, window, cx| {
                            this.navigate(Navigation::OpenFile(path), window, cx)
                        });
                    }
                }
                Ok(Ok(None)) => {} // Native cancellation leaves the active database intact.
                _ => {
                    let _ = client.update_in(cx, |this, _, cx| {
                        this.error = Some("file_io_error");
                        cx.notify();
                    });
                }
            }
        }));
    }

    pub(super) fn commit_file_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let password = Zeroizing::new(self.file_password.read(cx).value().to_string());
        if password.is_empty() {
            self.error = Some("file_empty_password");
            cx.notify();
            return;
        }
        if let Some(Form::OpenFile(path)) = &self.form {
            let path = path.clone();
            self.file_password = input("", true, window, cx);
            self.run_io(
                window,
                cx,
                move |service| service.open_file(&path, password.as_bytes()),
                Self::accept_file_session,
            );
            return;
        }
        if password.as_str() != self.file_confirmation.read(cx).value().as_ref() {
            self.error = Some("file_password_mismatch");
            cx.notify();
            return;
        }
        let name = self.form_input.read(cx).value().to_string();
        if name.is_empty() {
            self.error = Some("error");
            cx.notify();
            return;
        }
        let directory = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("."));
        let destination = cx.prompt_for_new_path(&directory, Some("Database.taypeer"));
        self.file_password = input("", true, window, cx);
        self.file_confirmation = input("", true, window, cx);
        self.io_task = Some(
            cx.spawn_in(window, async move |client, cx| match destination.await {
                Ok(Ok(Some(path))) => {
                    let _ = client.update_in(cx, |this, window, cx| {
                        if !matches!(this.form, Some(Form::Database)) || this.busy {
                            return;
                        }
                        this.run_io(
                            window,
                            cx,
                            move |service| service.create_file(&path, name, password.as_bytes()),
                            Self::accept_file_session,
                        );
                    });
                }
                Ok(Ok(None)) => {}
                _ => {
                    let _ = client.update_in(cx, |this, _, cx| {
                        this.error = Some("file_io_error");
                        cx.notify();
                    });
                }
            }),
        );
    }
}

pub(super) fn file_error(error: ServiceError) -> &'static str {
    use taypeer_services::StorageError;
    match error {
        ServiceError::Storage(StorageError::Authentication) => "file_auth_error",
        ServiceError::Storage(StorageError::EmptyPassword) => "file_empty_password",
        ServiceError::Storage(StorageError::UnsupportedVersion) => "file_version_error",
        ServiceError::Storage(StorageError::Busy) => "file_busy_error",
        ServiceError::Storage(StorageError::AlreadyExists) => "file_exists_error",
        ServiceError::Storage(StorageError::Changed | StorageError::CommitUncertain) => {
            "file_reopen_error"
        }
        ServiceError::Storage(StorageError::InvalidFile | StorageError::TooLarge)
        | ServiceError::InvalidDocument => "file_invalid_error",
        ServiceError::Storage(StorageError::Io | StorageError::Random) => "file_io_error",
        _ => "error",
    }
}
