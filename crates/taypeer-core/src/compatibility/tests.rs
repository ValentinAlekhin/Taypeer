use super::*;

#[test]
fn read_write_and_receipt_are_independent() {
    let schema = SchemaDescriptor::current();
    let read = schema.required_read_features();
    let write: BTreeSet<_> = schema
        .required_write_features()
        .iter()
        .filter(|id| id.as_str() != "taypeer.binary")
        .cloned()
        .collect();
    let old = ClientCapabilities::default().restricted(read, &write);
    let report = old.assess(&schema);
    assert!(report.read.is_supported());
    assert_eq!(
        report.write,
        CompatibilityAccess::MissingFeatures {
            features: BTreeSet::from([FeatureId::new("taypeer.binary").unwrap()])
        }
    );
    assert!(report.receive.is_supported());
    let blind = ClientCapabilities::default()
        .restricted(&BTreeSet::new(), read)
        .assess(&schema);
    assert!(!blind.read.is_supported());
    assert_eq!(blind.write, CompatibilityAccess::ReadingUnavailable);
    assert!(blind.receive.is_supported());
    assert!(
        ClientCapabilities::default()
            .assess(&schema)
            .write
            .is_supported()
    );
}

#[test]
fn unknown_features_and_schemas_require_the_right_update() {
    let base = SchemaDescriptor::current();
    let mut writes = base.required_write_features().clone();
    writes.insert(FeatureId::new("future.retention").unwrap());
    let schema =
        SchemaDescriptor::new(5, base.required_read_features().clone(), writes.clone()).unwrap();
    let report = ClientCapabilities::default().assess(&schema);
    assert!(report.read.is_supported());
    assert!(!report.write.is_supported());
    let schema = SchemaDescriptor::new(5, writes.clone(), writes).unwrap();
    assert!(
        !ClientCapabilities::default()
            .assess(&schema)
            .read
            .is_supported()
    );
    let future = SchemaDescriptor::new(99, BTreeSet::new(), BTreeSet::new()).unwrap();
    let report = ClientCapabilities::default().assess(&future);
    assert_eq!(
        report.read,
        CompatibilityAccess::UnsupportedSchema { schema_version: 99 }
    );
    assert!(report.receive.is_supported());
}

#[test]
fn deserialization_cannot_drop_mandatory_semantics_or_accept_unbounded_identifiers() {
    let schema = SchemaDescriptor::current();
    let encoded = serde_json::to_value(&schema).unwrap();
    assert_eq!(
        serde_json::from_value::<SchemaDescriptor>(encoded.clone()).unwrap(),
        schema
    );
    for field in ["required_read_features", "required_write_features"] {
        let mut value = encoded.clone();
        value[field] = serde_json::json!([]);
        assert!(serde_json::from_value::<SchemaDescriptor>(value).is_err());
    }
    let mut value = encoded.clone();
    value["schema_version"] = serde_json::json!(0);
    assert!(serde_json::from_value::<SchemaDescriptor>(value).is_err());
    let mut value = encoded;
    value["required_read_features"][0] = serde_json::json!("PUBLIC\nsecret");
    assert!(serde_json::from_value::<SchemaDescriptor>(value).is_err());
    assert!(FeatureId::new("x".repeat(97)).is_err());
    let many: BTreeSet<_> = (0..129)
        .map(|i| FeatureId::new(format!("future.{i}")).unwrap())
        .collect();
    assert!(SchemaDescriptor::new(99, many, BTreeSet::new()).is_err());
}
