//! A new schema-one PUBLIC image corpus; the historical Android capture is immutable.
use super::*;
use crate::{BinaryEdit, BinaryRequest, BinaryTarget, IconInput};
use taypeer_core::OperationId;

const EAGER_PASSWORD: &[u8] = b"PUBLIC image list master";
const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="#14dc28"/></svg>"##;

/// Deliberately explicit generation into a new directory, never an ordinary test setup.
#[test]
#[ignore = "PUBLIC schema-one eager corpus generator; requires TAYPEER_EAGER_CORPUS_OUTPUT"]
fn generate_public_eager_corpus() {
    let output = PathBuf::from(
        std::env::var_os("TAYPEER_EAGER_CORPUS_OUTPUT").expect("explicit new output directory"),
    );
    std::fs::create_dir(&output).unwrap();
    let temporary = tempfile::tempdir().unwrap();
    let path = temporary.path().join("PUBLIC eager.taypeer");
    let profile = Profile::new(71);
    let seed = DatabaseService::prepare_managed(
        "PUBLIC image list visibility".into(),
        EAGER_PASSWORD,
        &profile.author(),
        profile.identity(),
        1,
        DatabasePolicy::new(10 * 1024 * 1024, 100 * 1024 * 1024, 500).unwrap(),
    )
    .unwrap();
    drop(seed.create(&path, &profile.transport, None).unwrap());
    let (mut service, session) = profile.open(path.clone(), EAGER_PASSWORD);
    let group = service
        .create_group(
            &session,
            "PUBLIC image list group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    let image = temporary.path().join("PUBLIC rectangle.svg");
    std::fs::write(&image, SVG).unwrap();
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: BinaryTarget::Group(group.clone()),
                edit: BinaryEdit::Icon(IconInput::File(image.clone())),
                review: None,
            },
            &OperationId::new("PUBLIC eager group image"),
        )
        .unwrap();
    service.start_create_entry(&session, group.clone()).unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC image 000".into(),
                ..Default::default()
            },
        )
        .unwrap();
    service
        .edit_binary(
            &session,
            &BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Icon(IconInput::File(image)),
                review: None,
            },
            &OperationId::new("PUBLIC eager entry image"),
        )
        .unwrap();
    let first = service
        .save_draft(&session, &crate::new_operation_id().unwrap())
        .unwrap()
        .value;
    for index in 1..70 {
        service
            .clone_entry(
                &session,
                &first,
                group.clone(),
                Some(format!("PUBLIC image {index:03}")),
                &OperationId::new(format!("PUBLIC eager clone {index:03}")),
            )
            .unwrap();
    }
    assert_eq!(service.entries(&session, None, "").unwrap().value.len(), 70);
    assert!(service.drafts(&session).unwrap().value.is_empty());
    service.lock(&session).unwrap();
    let destination = output.join("seventy-image-entries.taypeer");
    std::fs::copy(path, &destination).unwrap();
    let bytes = std::fs::read(&destination).unwrap();
    let digest = Digest::of(&bytes);
    let manifest = serde_json::json!({
        "classification": "PUBLIC synthetic schema-one fixture",
        "schema": taypeer_core::CURRENT_SCHEMA,
        "source": "crates/taypeer-services/src/managed/tests/eager_corpus.rs",
        "generator": "managed::tests::eager_corpus::generate_public_eager_corpus",
        "password": "PUBLIC image list master",
        "author_seed": "PUBLIC [71;32]", "transport_seed": "PUBLIC [111;32]",
        "bytes": bytes.len(), "sha256": digest.to_string(),
        "entries": 70, "group": "PUBLIC image list group", "image_bytes": SVG.len(),
        "native_android_acceptance": false,
    });
    std::fs::write(
        output.join("manifest.json"),
        serde_json::to_vec_pretty(&manifest).unwrap(),
    )
    .unwrap();
    std::fs::write(
        output.join("SHA256SUMS"),
        format!("{digest}  seventy-image-entries.taypeer\n"),
    )
    .unwrap();
    assert!(destination.is_file());
}
