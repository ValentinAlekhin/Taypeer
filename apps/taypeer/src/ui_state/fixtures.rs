//! Public, deterministic samples. Nothing here originates from a user's vault.

use super::*;

pub(super) fn catalog() -> CatalogStore {
    let mut catalog = CatalogStore::new();
    let work = catalog
        .create_database("Work.taypeer".into(), "Public UI sample".into())
        .expect("valid fixture");
    let group = catalog
        .save_group(
            work,
            None,
            None,
            "Work".into(),
            String::new(),
            "folder".into(),
        )
        .expect("valid fixture");
    catalog
        .save_group(
            work,
            None,
            Some(group),
            "Services".into(),
            String::new(),
            "folder".into(),
        )
        .expect("valid fixture");
    catalog
        .save_group(
            work,
            None,
            Some(group),
            "Infrastructure".into(),
            String::new(),
            "server".into(),
        )
        .expect("valid fixture");
    catalog
        .save_group(
            work,
            None,
            None,
            "Archive".into(),
            String::new(),
            "folder".into(),
        )
        .expect("valid fixture");
    for name in [
        "Cloudflare",
        "Figma",
        "GitHub",
        "Linear",
        "Notion",
        "Vercel",
    ] {
        let mut content = EntryContent {
            title: name.into(),
            username: "demo@example.test".into(),
            password: "PUBLIC-UI-SAMPLE-42".into(),
            url: format!("https://{}.example.test", name.to_lowercase()),
            tags: "work, development".into(),
            notes: "Public synthetic account for UI review.".into(),
            icon: "key-round".into(),
            ..Default::default()
        };
        if name == "GitHub" {
            content.attributes = vec![
                Attribute {
                    key: "Recovery code".into(),
                    value: "PUBLIC-RECOVERY-123".into(),
                    protected: true,
                },
                Attribute {
                    key: "Organization".into(),
                    value: "Example Studio".into(),
                    protected: false,
                },
            ];
            content.attachments = vec![Attachment {
                name: "recovery-codes.txt".into(),
                bytes: 20480,
            }];
        }
        let entry = catalog
            .save_entry(work, group, None, content.clone())
            .expect("valid fixture");
        if name == "GitHub" {
            content.notes =
                "Public synthetic account.\nA second saved revision for comparing changes.".into();
            content.password = "PUBLIC-UI-UPDATED-84".into();
            catalog
                .save_entry(work, group, Some(entry), content)
                .expect("valid fixture");
        }
    }
    let personal = catalog
        .create_database("Personal.taypeer".into(), "Second public UI sample".into())
        .expect("valid fixture");
    let group = catalog
        .save_group(
            personal,
            None,
            None,
            "Personal".into(),
            String::new(),
            "folder".into(),
        )
        .expect("valid fixture");
    catalog
        .save_entry(
            personal,
            group,
            None,
            EntryContent {
                title: "A very long account title for checking truncation and narrow columns"
                    .into(),
                username: "long-public-username@example.test".into(),
                password: "PUBLIC-PERSONAL-ONLY".into(),
                notes: "Find this record in all unlocked databases.".into(),
                icon: "globe".into(),
                ..Default::default()
            },
        )
        .expect("valid fixture");
    catalog
        .create_database("Empty.taypeer".into(), "Empty public UI sample".into())
        .expect("valid fixture");
    catalog
}
