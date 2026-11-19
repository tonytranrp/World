//! The material table: every definition, validated, with O(1) lookup by id.

use std::fmt;

use serde::{Deserialize, Serialize};

use super::{MaterialDef, MaterialId};

/// One reason a table was rejected.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterialError {
    /// A field of one material is out of range.
    Field {
        id: u16,
        name: String,
        field: &'static str,
        problem: String,
    },
    DuplicateId(u16),
    DuplicateName(String),
}

impl fmt::Display for MaterialError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Field {
                id,
                name,
                field,
                problem,
            } => write!(f, "material {id} ({name:?}): {field} {problem}"),
            Self::DuplicateId(id) => write!(f, "material id {id} is used more than once"),
            Self::DuplicateName(name) => write!(f, "material name {name:?} is used more than once"),
        }
    }
}

/// Every reason a table was rejected, so one reload reports all mistakes at once.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MaterialErrors(pub Vec<MaterialError>);

impl fmt::Display for MaterialErrors {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (i, e) in self.0.iter().enumerate() {
            if i > 0 {
                f.write_str("; ")?;
            }
            write!(f, "{e}")?;
        }
        Ok(())
    }
}

impl std::error::Error for MaterialErrors {}

/// The on-disk shape: just the list. Validation turns it into a table.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct MaterialTableSpec {
    materials: Vec<MaterialDef>,
}

/// All materials, validated. Immutable: a change replaces the whole table.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "MaterialTableSpec", into = "MaterialTableSpec")]
pub struct MaterialTable {
    materials: Vec<MaterialDef>,
    /// `slots[id]` is the index into `materials`, or `u32::MAX` when unused.
    slots: Vec<u32>,
}

const EMPTY: u32 = u32::MAX;

impl MaterialTable {
    /// Validates `materials` and builds the lookup. Reports every problem, not just the first.
    pub fn new(materials: Vec<MaterialDef>) -> Result<Self, MaterialErrors> {
        let mut errors = Vec::new();
        for m in &materials {
            for (field, problem) in m.range_problems() {
                errors.push(MaterialError::Field {
                    id: m.id.0,
                    name: m.name.clone(),
                    field,
                    problem,
                });
            }
        }
        let mut names: Vec<&str> = materials.iter().map(|m| m.name.as_str()).collect();
        names.sort_unstable();
        for pair in names.windows(2) {
            if pair[0] == pair[1] && !errors.contains(&MaterialError::DuplicateName(pair[0].into()))
            {
                errors.push(MaterialError::DuplicateName(pair[0].into()));
            }
        }

        let len = materials
            .iter()
            .map(|m| m.id.0 as usize + 1)
            .max()
            .unwrap_or(1);
        let mut slots = vec![EMPTY; len];
        for (index, m) in materials.iter().enumerate() {
            let slot = &mut slots[m.id.0 as usize];
            if *slot != EMPTY {
                if !errors.contains(&MaterialError::DuplicateId(m.id.0)) {
                    errors.push(MaterialError::DuplicateId(m.id.0));
                }
            } else {
                *slot = index as u32;
            }
        }

        if errors.is_empty() {
            Ok(Self { materials, slots })
        } else {
            Err(MaterialErrors(errors))
        }
    }

    /// The definition for `id`, or `None` for air and unknown ids.
    pub fn get(&self, id: MaterialId) -> Option<&MaterialDef> {
        match self.slots.get(id.0 as usize) {
            Some(&index) if index != EMPTY => Some(&self.materials[index as usize]),
            _ => None,
        }
    }

    pub fn by_name(&self, name: &str) -> Option<&MaterialDef> {
        self.materials.iter().find(|m| m.name == name)
    }

    /// Definitions in file order.
    pub fn iter(&self) -> impl Iterator<Item = &MaterialDef> {
        self.materials.iter()
    }

    pub fn len(&self) -> usize {
        self.materials.len()
    }

    pub fn is_empty(&self) -> bool {
        self.materials.is_empty()
    }
}

impl Default for MaterialTable {
    /// An empty table: everything but air is unknown.
    fn default() -> Self {
        Self {
            materials: Vec::new(),
            slots: vec![EMPTY],
        }
    }
}

/// Validation happens while parsing ([`MaterialTable::new`]), so a parsed
/// table needs no further checks.
impl crate::config::ConfigAsset for MaterialTable {
    const NAME: &'static str = "materials";
    const PATH: &'static str = "config/materials.ron";
}

impl TryFrom<MaterialTableSpec> for MaterialTable {
    type Error = MaterialErrors;
    fn try_from(spec: MaterialTableSpec) -> Result<Self, Self::Error> {
        Self::new(spec.materials)
    }
}

impl From<MaterialTable> for MaterialTableSpec {
    fn from(table: MaterialTable) -> Self {
        Self {
            materials: table.materials,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::def::sample;
    use super::*;

    #[test]
    fn lookup_by_id_and_name() {
        let t = MaterialTable::new(vec![sample(3, "dirt"), sample(1, "stone")]).unwrap();
        assert_eq!(t.get(MaterialId(1)).unwrap().name, "stone");
        assert_eq!(t.get(MaterialId(3)).unwrap().name, "dirt");
        assert!(t.get(MaterialId(2)).is_none());
        assert!(t.get(MaterialId::AIR).is_none());
        assert!(t.get(MaterialId(999)).is_none());
        assert_eq!(t.by_name("dirt").unwrap().id, MaterialId(3));
        assert_eq!(t.len(), 2);
    }

    #[test]
    fn duplicates_are_reported_once_each() {
        let err = MaterialTable::new(vec![
            sample(1, "stone"),
            sample(1, "stone"),
            sample(1, "stone"),
        ])
        .unwrap_err();
        assert_eq!(
            err.0,
            [
                MaterialError::DuplicateName("stone".into()),
                MaterialError::DuplicateId(1)
            ]
        );
    }

    #[test]
    fn every_problem_is_reported_together() {
        let mut bad = sample(2, "glass");
        bad.roughness = 2.0;
        bad.restitution = -1.0;
        let err = MaterialTable::new(vec![sample(1, "stone"), bad]).unwrap_err();
        assert_eq!(err.0.len(), 2);
        let text = err.to_string();
        assert!(
            text.contains("roughness") && text.contains("restitution"),
            "{text}"
        );
    }

    #[test]
    fn the_default_table_is_empty() {
        let t = MaterialTable::default();
        assert!(t.is_empty());
        assert!(t.get(MaterialId(1)).is_none());
    }
}
