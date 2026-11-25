use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Compartment(String);

impl Compartment {
    pub fn new(name: impl Into<String>) -> Self {
        Self(name.into())
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// A set of compartments. The empty label is public data.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Label(BTreeSet<Compartment>);

impl Label {
    pub fn public() -> Self {
        Self(BTreeSet::new())
    }

    pub fn new(compartments: impl IntoIterator<Item = Compartment>) -> Self {
        Self(compartments.into_iter().collect())
    }

    pub fn is_subset(&self, other: &Label) -> bool {
        self.0.is_subset(&other.0)
    }

    /// Compartments in ascending order.
    pub fn compartments(&self) -> impl Iterator<Item = &Compartment> {
        self.0.iter()
    }

    #[must_use]
    pub fn meet(&self, other: &Label) -> Label {
        Self(self.0.intersection(&other.0).cloned().collect())
    }
}
