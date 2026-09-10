//! Dense positions with validated library indices and identity-aware collision handling.

use fractional_index::FractionalIndex;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::cmp::Ordering;

/// Invalid encoding, bounds, or exhausted input budget. Never contains user data.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct OrderError;

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
struct Segment(FractionalIndex, String);

/// A bounded dense position. Compare sibling positions with `compare`, supplying their IDs.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OrderKey(Vec<Segment>);

const MAX_BYTES: usize = 4096;

impl OrderKey {
    /// Allocate between two siblings, including siblings with identical stored positions.
    /// Missing bounds represent the beginning/end. Existing positions never change.
    pub fn between(
        left: Option<(&Self, &str)>,
        right: Option<(&Self, &str)>,
        operation: &str,
    ) -> Result<Self, OrderError> {
        if operation.is_empty() {
            return Err(OrderError);
        }
        let left = left.map(|(key, id)| key.complete(id));
        let right = right.map(|(key, id)| key.complete(id));
        let segment = |rank| Segment(rank, operation.to_owned());
        let path = match (left, right) {
            (None, None) => vec![segment(FractionalIndex::default())],
            (None, Some(right)) => vec![segment(FractionalIndex::new_before(&right[0].0))],
            (Some(left), None) => vec![segment(FractionalIndex::new_after(&left[0].0))],
            (Some(left), Some(right)) => {
                if left >= right {
                    return Err(OrderError);
                }
                let common = left.iter().zip(&right).take_while(|(a, b)| a == b).count();
                if common == left.len() {
                    let mut path = left;
                    path.push(segment(FractionalIndex::new_before(&right[common].0)));
                    path
                } else if left[common].0 < right[common].0 {
                    let mut path = left[..common].to_vec();
                    path.push(segment(
                        FractionalIndex::new_between(&left[common].0, &right[common].0)
                            .ok_or(OrderError)?,
                    ));
                    path
                } else {
                    let mut path = left;
                    path.push(segment(FractionalIndex::default()));
                    path
                }
            }
        };
        let result = Self(path);
        result.validate()?;
        Ok(result)
    }

    fn complete(&self, id: &str) -> Vec<Segment> {
        let mut path = self.0.clone();
        path.push(Segment(FractionalIndex::default(), id.to_owned()));
        path
    }

    /// Compare positions, using stable object identity when positions collide.
    pub fn compare(&self, id: &str, other: &Self, other_id: &str) -> Ordering {
        self.complete(id).cmp(&other.complete(other_id))
    }

    fn validate(&self) -> Result<(), OrderError> {
        if self.0.is_empty()
            || self.0.len() > MAX_BYTES
            || self.0.iter().any(|s| s.1.is_empty())
            || self.0.iter().fold(0usize, |total, s| {
                total
                    .saturating_add(s.0.as_bytes().len())
                    .saturating_add(s.1.len())
            }) > MAX_BYTES
        {
            return Err(OrderError);
        }
        Ok(())
    }
}

impl Serialize for OrderKey {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0
            .iter()
            .map(|s| (s.0.as_bytes(), &s.1))
            .collect::<Vec<_>>()
            .serialize(serializer)
    }
}
impl<'de> Deserialize<'de> for OrderKey {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = Vec::<(Vec<u8>, String)>::deserialize(deserializer)?;
        let mut segments = Vec::new();
        for (bytes, identity) in raw {
            let rank = FractionalIndex::from_bytes(bytes)
                .map_err(|_| serde::de::Error::custom("invalid order key"))?;
            segments.push(Segment(rank, identity));
        }
        let result = Self(segments);
        result
            .validate()
            .map_err(|_| serde::de::Error::custom("invalid order key"))?;
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn inserts_between_collisions_without_rewriting_neighbours() {
        let same = OrderKey::between(None, None, "PUBLIC original").unwrap();
        let mut items = vec![
            (same.clone(), "PUBLIC a".to_owned()),
            (same, "PUBLIC z".to_owned()),
        ];
        for n in 0..400 {
            let position = (n * 17 + 1) % (items.len() + 1);
            let left = position
                .checked_sub(1)
                .map(|i| (&items[i].0, items[i].1.as_str()));
            let right = items.get(position).map(|(k, id)| (k, id.as_str()));
            let id = format!("PUBLIC {n}");
            let key = OrderKey::between(left, right, &id).unwrap();
            if let Some((l, lid)) = left {
                assert!(l.compare(lid, &key, &id).is_lt());
            }
            if let Some((r, rid)) = right {
                assert!(key.compare(&id, r, rid).is_lt());
            }
            items.insert(position, (key, id));
        }
    }
}
