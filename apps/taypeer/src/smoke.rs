//! Finite end-to-end check of the Rust demonstration service, without a window.

use taypeer_services::{DEMO_PASSWORD, DemoService, EditableAttribute, EditableEntry};

pub fn run() {
    match exercise() {
        Ok(()) => {
            println!("Taypeer demo smoke passed: create, edit, history, search, lock and restore.")
        }
        Err(error) => {
            eprintln!("Taypeer demo smoke failed: {error}");
            std::process::exit(1);
        }
    }
}

fn exercise() -> Result<(), Box<dyn std::error::Error>> {
    let mut service = DemoService::with_clock(|| 1_789_000_000_123);
    let database = service.create_database("PUBLIC smoke / Проверка")?;
    let session = service.unlock(&database, DEMO_PASSWORD)?;
    ensure(
        service.groups(&session)?.value.is_empty(),
        "new database is not empty",
    )?;
    let group = service
        .create_group(&session, "PUBLIC group / Группа".into(), None)?
        .value;
    let child = service
        .create_group(&session, "PUBLIC child".into(), Some(group.id.clone()))?
        .value;
    ensure(
        child.parent == Some(group.id.clone()),
        "nested group has the wrong parent",
    )?;
    service.start_create_entry(&session, group.id.clone())?;
    service.update_draft(
        &session,
        EditableEntry {
            title: "PUBLIC smoke entry".into(),
            username: Some("".into()),
            password: Some("PUBLIC_PASSWORD_SENTINEL".into()),
            notes: Some("  Точный текст é  ".into()),
            tags: vec!["PUBLIC tag".into()],
            expires_at: Some(1_890_000_000_123),
            attributes: vec![EditableAttribute {
                id: None,
                name: "PUBLIC protected attribute".into(),
                value: "PUBLIC_ATTRIBUTE_SENTINEL".into(),
                protected: true,
            }],
            ..EditableEntry::default()
        },
    )?;
    let entry = service.save_draft(&session)?.value;
    ensure(
        service.history(&session, &entry)?.value.len() == 1,
        "create did not record one revision",
    )?;
    let view = service.view_entry(&session, &entry)?.value;
    ensure(view.username == Some(String::new()), "empty value was lost")?;
    ensure(
        view.attributes[0].value.is_none(),
        "ordinary view disclosed a protected value",
    )?;
    ensure(
        service
            .entries(&session, None, "PUBLIC_PASSWORD_SENTINEL")?
            .value
            .is_empty(),
        "password entered search",
    )?;
    ensure(
        service
            .entries(&session, None, "PUBLIC_ATTRIBUTE_SENTINEL")?
            .value
            .is_empty(),
        "protected attribute entered search",
    )?;
    ensure(
        service.entries(&session, None, "точный")?.value.len() == 1,
        "ordinary Unicode search failed",
    )?;
    let mut draft = service.start_edit_entry(&session, &entry)?.value.fields;
    draft.title = "PUBLIC updated entry".into();
    service.update_draft(&session, draft)?;
    service.save_draft(&session)?;
    ensure(
        service.history(&session, &entry)?.value.len() == 2,
        "edit did not record one revision",
    )?;
    let delayed = service.reveal_password(&session, &entry)?;
    let mut interrupted = service.start_edit_entry(&session, &entry)?.value.fields;
    interrupted.notes = Some("PUBLIC interrupted draft".into());
    service.update_draft(&session, interrupted)?;
    service.lock(&session)?;
    ensure(
        !service.is_current(&delayed.session),
        "locked session accepted an old response",
    )?;
    ensure(
        service.view_entry(&session, &entry).is_err(),
        "locked document is readable",
    )?;
    let reopened = service.unlock(&database, DEMO_PASSWORD)?;
    ensure(
        !service.is_current(&delayed.session),
        "reopening revived an old response",
    )?;
    ensure(
        service.draft(&reopened)?.value.is_none(),
        "interrupted draft restored without consent",
    )?;
    ensure(
        service.pending_draft(&reopened)?.value.is_some(),
        "interrupted draft was lost",
    )?;
    let restored = service.restore_draft(&reopened)?;
    ensure(
        restored.session == reopened,
        "restored draft has an old session",
    )?;
    ensure(
        restored.value.fields.notes.as_deref() == Some("PUBLIC interrupted draft"),
        "restored draft changed its text",
    )?;
    service.cancel_draft(&reopened)?;
    ensure(
        service.history(&reopened, &entry)?.value.len() == 2,
        "cancel created a revision",
    )?;
    let second = service.create_database("PUBLIC second database")?;
    let second_session = service.unlock(&second, DEMO_PASSWORD)?;
    ensure(
        service.view_entry(&second_session, &entry).is_err(),
        "entry crossed database boundary",
    )?;
    service.lock_all();
    ensure(
        !service.is_current(&reopened) && !service.is_current(&second_session),
        "lock_all left an open session",
    )?;
    Ok(())
}

fn ensure(condition: bool, message: &'static str) -> Result<(), Box<dyn std::error::Error>> {
    if condition {
        Ok(())
    } else {
        Err(message.into())
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn headless_demo_runs_the_complete_service_scenario() {
        super::exercise().expect("the public synthetic scenario must succeed");
    }
}
