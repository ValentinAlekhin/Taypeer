//! Checked access to the experimental Automerge representation.

use super::Error;
use automerge::{ObjId, ObjType, ReadDoc, ScalarValue, Value};
use serde::{Serialize, de::DeserializeOwned};

pub(super) fn encode<T: Serialize>(value: &T) -> Result<ScalarValue, Error> {
    serde_json::to_vec(value)
        .map(ScalarValue::Bytes)
        .map_err(|_| Error::InvalidDocument)
}

pub(super) fn decode<T: DeserializeOwned>(value: &Value<'_>) -> Result<T, Error> {
    let Value::Scalar(scalar) = value else {
        return Err(Error::InvalidDocument);
    };
    let ScalarValue::Bytes(bytes) = scalar.as_ref() else {
        return Err(Error::InvalidDocument);
    };
    serde_json::from_slice(bytes).map_err(|_| Error::InvalidDocument)
}

pub(super) fn unique_optional<R: ReadDoc>(
    read: &R,
    obj: &ObjId,
    key: &str,
) -> Result<Option<Value<'static>>, Error> {
    let values = read.get_all(obj, key)?;
    match values.len() {
        0 => Ok(None),
        1 => Ok(values
            .into_iter()
            .next()
            .map(|(value, _)| value.into_owned())),
        _ => Err(Error::Conflict),
    }
}

pub(super) fn unique<R: ReadDoc>(
    read: &R,
    obj: &ObjId,
    key: &str,
) -> Result<Value<'static>, Error> {
    unique_optional(read, obj, key)?.ok_or(Error::NotFound)
}

pub(super) fn object<R: ReadDoc>(read: &R, obj: &ObjId, key: &str) -> Result<ObjId, Error> {
    let values = read.get_all(obj, key)?;
    if values.len() > 1 {
        return Err(Error::DuplicateId);
    }
    let (value, id) = values.into_iter().next().ok_or(Error::NotFound)?;
    if value != Value::Object(ObjType::Map) {
        return Err(Error::InvalidDocument);
    }
    Ok(id)
}
