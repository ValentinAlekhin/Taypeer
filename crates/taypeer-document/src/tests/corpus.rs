//! Frozen PUBLIC unknown-field document and its explicit generator.
use super::*;

fn extension_fixture() -> std::path::PathBuf {
    std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/fixtures/dev5/extensions.automerge")
}

#[test]
#[ignore = "explicit PUBLIC corpus generator; requires TAYPEER_CORPUS_OUTPUT"]
fn generate_extension_corpus() {
    use std::io::Write;
    let output = std::path::PathBuf::from(
        std::env::var_os("TAYPEER_CORPUS_OUTPUT").expect("explicit output directory"),
    );
    let (mut document, _, entry) = setup();
    let entries = object(&document.doc, &ROOT, "entries").unwrap();
    let entry_object = object(&document.doc, &entries, entry.as_str()).unwrap();
    let mut tx = document.doc.transaction();
    tx.put(ROOT, "future_optional", "PUBLIC retained root extension")
        .unwrap();
    tx.put(
        &entry_object,
        "future_extension",
        "PUBLIC retained entry extension",
    )
    .unwrap();
    tx.commit();
    let mut other = document.fork();
    for (branch, value) in [
        (&mut document, "PUBLIC left extension"),
        (&mut other, "PUBLIC right extension"),
    ] {
        let mut tx = branch.doc.transaction();
        tx.put(ROOT, "future_conflict", value).unwrap();
        tx.commit();
    }
    document.merge(&other).unwrap();
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(output.join("extensions.automerge"))
        .unwrap();
    file.write_all(&document.export()).unwrap();
}

#[test]
fn frozen_unknown_fields_and_conflicts_survive_addressed_edit_and_reload() {
    let original = std::fs::read(extension_fixture()).unwrap();
    let mut document = Document::load(&original).unwrap();
    let entry = document.entries().unwrap()[0].id.clone();
    let database = document.database_id().clone();
    let revisions: Vec<_> = document
        .history(&entry)
        .unwrap()
        .into_iter()
        .map(|r| r.id)
        .collect();
    let mut draft = document.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().username = Some("PUBLIC edited login".into());
    document.save_entry(draft, 2000).unwrap();
    let reopened = Document::load(&document.export()).unwrap();
    assert_eq!(reopened.database_id(), &database);
    assert_eq!(
        reopened
            .doc
            .get(ROOT, "future_optional")
            .unwrap()
            .unwrap()
            .0
            .to_str(),
        Some("PUBLIC retained root extension")
    );
    let entries = object(&reopened.doc, &ROOT, "entries").unwrap();
    let entry_object = object(&reopened.doc, &entries, entry.as_str()).unwrap();
    assert_eq!(
        reopened
            .doc
            .get(entry_object, "future_extension")
            .unwrap()
            .unwrap()
            .0
            .to_str(),
        Some("PUBLIC retained entry extension")
    );
    let variants: BTreeSet<_> = reopened
        .doc
        .get_all(ROOT, "future_conflict")
        .unwrap()
        .into_iter()
        .map(|(v, _)| v.to_str().unwrap().to_owned())
        .collect();
    assert_eq!(
        variants,
        BTreeSet::from([
            "PUBLIC left extension".to_owned(),
            "PUBLIC right extension".to_owned()
        ])
    );
    let history = reopened.history(&entry).unwrap();
    assert_eq!(history.len(), revisions.len() + 1);
    for id in revisions {
        assert!(history.iter().any(|r| r.id == id));
    }
    assert_eq!(std::fs::read(extension_fixture()).unwrap(), original);
}
