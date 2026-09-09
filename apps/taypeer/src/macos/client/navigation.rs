//! Session lifecycle and navigation guards; views delegate transitions here.

use super::{Client, EntryTab, Navigation};
use crate::macos::common::input;
use gpui_kit::component::WindowExt;
use gpui_kit::component::input::InputEvent;
use gpui_kit::*;
use taypeer_services::SessionToken;

impl Client {
    pub(super) fn accepts(&self, token: &SessionToken) -> bool {
        self.session.as_ref() == Some(token) && self.service.is_current(token)
    }

    pub(super) fn clear_content(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.selected = None;
        self.group = None;
        self.revealed.clear();
        self.revision = None;
        self.pending = None;
        self.restore = false;
        self.tab = EntryTab::Overview;
        self.form = None;
        self.password = input("", true, window, cx);
        self.form_input = input("", false, window, cx);
        self.search = input("", false, window, cx);
        self.subscriptions._search =
            cx.subscribe_in(&self.search, window, |_, _, _: &InputEvent, _, cx| {
                cx.notify()
            });
        self.bind_prompt_inputs(window, cx);
        window.close_all_dialogs(cx);
        self.root_focus.focus(window, cx);
    }

    pub(super) fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = self.session.take() {
            self.sessions.remove(&token.database);
            if self.service.lock(&token).is_err() {
                self.error = Some("error");
            }
        }
        self.clear_content(window, cx);
        cx.notify();
    }

    pub(super) fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(id) = &self.database else {
            return;
        };
        match self
            .service
            .unlock(id, self.password.read(cx).value().as_ref())
        {
            Ok(token) => {
                self.password = input("", true, window, cx);
                self.restore = self
                    .service
                    .pending_draft(&token)
                    .is_ok_and(|v| self.service.is_current(&v.session) && v.value.is_some());
                self.sessions.insert(token.database.clone(), token.clone());
                self.session = Some(token);
                self.error = None;
                self.bind_prompt_inputs(window, cx);
                if self.restore {
                    self.modal_focus.focus(window, cx);
                } else {
                    self.root_focus.focus(window, cx);
                }
            }
            Err(_) => self.error = Some("password_error"),
        }
        cx.notify();
    }

    pub(super) fn navigate(
        &mut self,
        action: Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.as_ref().is_some_and(|e| e.draft.dirty) {
            self.pending = Some(action);
            self.modal_focus.focus(window, cx);
            cx.notify();
            return;
        }
        self.perform_navigation(action, window, cx);
    }

    pub(super) fn perform_navigation(
        &mut self,
        action: Navigation,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.editor.is_some()
            && let Some(token) = &self.session
        {
            let _ = self.service.cancel_draft(token);
        }
        self.editor = None;
        self.revealed.clear();
        self.revision = None;
        self.tab = EntryTab::Overview;
        self.pending = None;
        self.error = None;
        self.root_focus.focus(window, cx);
        match action {
            Navigation::Database(id) => {
                self.clear_content(window, cx);
                self.session = self
                    .sessions
                    .get(&id)
                    .filter(|token| self.service.is_current(token))
                    .cloned();
                self.restore = self.session.as_ref().is_some_and(|token| {
                    self.service
                        .pending_draft(token)
                        .is_ok_and(|reply| reply.value.is_some())
                });
                self.database = Some(id);
            }
            Navigation::Group(id) => {
                self.group = Some(id);
                self.selected = None;
            }
            Navigation::Entry(id) => {
                self.selected = Some(id);
            }
            Navigation::NewEntry => {
                if let (Some(token), Some(group)) = (&self.session, &self.group) {
                    match self.service.start_create_entry(token, group.clone()) {
                        Ok(reply) if self.accepts(&reply.session) => {
                            self.selected = None;
                            self.install_editor(reply.session, reply.value, window, cx);
                        }
                        _ => self.error = Some("error"),
                    }
                }
            }
            Navigation::Cancel => {}
            Navigation::Form(form, value) => self.open_form(form, &value, window, cx),
        }
        cx.notify();
    }

    pub(super) fn save_and_navigate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.save(window, cx)
            && let Some(action) = self.pending.take()
        {
            self.perform_navigation(action, window, cx);
        }
    }

    pub(super) fn discard_and_navigate(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(action) = self.pending.take() {
            self.perform_navigation(action, window, cx);
        }
    }

    pub(super) fn stay_in_editor(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        self.pending = None;
        cx.notify();
    }

    pub(super) fn restore_draft(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(token) = &self.session {
            match self.service.restore_draft(token) {
                Ok(reply) if self.accepts(&reply.session) => {
                    let draft = reply.value;
                    self.group = Some(draft.group_id.clone());
                    self.selected = draft.entry_id.clone();
                    self.install_editor(reply.session, draft, window, cx);
                }
                _ => self.error = Some("error"),
            }
        }
        self.restore = false;
        cx.notify();
    }

    pub(super) fn discard_restored(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        if let Some(token) = &self.session
            && self.service.cancel_draft(token).is_err()
        {
            self.error = Some("error");
        }
        self.restore = false;
        cx.notify();
    }
}
