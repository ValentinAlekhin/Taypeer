//! Read-only service projections with one session freshness gate.

use super::Client;
use gpui_kit::*;
use std::collections::BTreeSet;
use taypeer_services::{EntrySummary, EntryView, GroupId, GroupSummary, RevisionSummary};

impl Client {
    pub(super) fn visible_groups(&self) -> Vec<GroupSummary> {
        let groups = self.groups();
        fn walk(
            groups: &[GroupSummary],
            parent: Option<&GroupId>,
            collapsed: &BTreeSet<GroupId>,
            out: &mut Vec<GroupSummary>,
        ) {
            for group in groups
                .iter()
                .filter(|group| group.parent.as_ref() == parent)
            {
                out.push(group.clone());
                if !collapsed.contains(&group.id) {
                    walk(groups, Some(&group.id), collapsed, out);
                }
            }
        }
        let mut result = Vec::new();
        walk(&groups, None, &self.collapsed, &mut result);
        result
    }

    pub(super) fn visible_entries(&self, cx: &App) -> Vec<EntrySummary> {
        let query = self.search.read(cx).value().to_string();
        let mut entries = self
            .session
            .as_ref()
            .and_then(|token| {
                self.service
                    .entries(
                        token,
                        if query.is_empty() {
                            self.group.as_ref()
                        } else {
                            None
                        },
                        &query,
                    )
                    .ok()
            })
            .filter(|reply| self.accepts(&reply.session))
            .map(|reply| reply.value)
            .unwrap_or_default();
        entries.sort_by(|a, b| a.title.cmp(&b.title).then(a.id.cmp(&b.id)));
        if self.descending {
            entries.reverse();
        }
        if query.is_empty() && self.group.is_none() {
            entries.clear();
        }
        entries
    }

    pub(super) fn groups(&self) -> Vec<GroupSummary> {
        self.session
            .as_ref()
            .and_then(|token| self.service.groups(token).ok())
            .filter(|reply| self.accepts(&reply.session))
            .map(|reply| reply.value)
            .unwrap_or_default()
    }

    pub(super) fn selected_view(&self) -> Option<EntryView> {
        self.session
            .as_ref()
            .and_then(|token| {
                self.selected.as_ref().and_then(|id| {
                    if let Some(revision) = &self.revision {
                        self.service.revision(token, id, revision).ok()
                    } else {
                        self.service.view_entry(token, id).ok()
                    }
                })
            })
            .filter(|r| self.accepts(&r.session))
            .map(|r| r.value)
    }

    pub(super) fn entry_history(&self) -> Vec<RevisionSummary> {
        self.session
            .as_ref()
            .and_then(|token| {
                self.selected
                    .as_ref()
                    .and_then(|id| self.service.history(token, id).ok())
            })
            .filter(|r| self.accepts(&r.session))
            .map(|r| r.value)
            .unwrap_or_default()
    }
}
