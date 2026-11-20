//! Material identifiers.

use serde::{Deserialize, Serialize};

/// Index into the material table. `0` is air: an empty voxel.
#[derive(
    Clone, Copy, Debug, Default, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
)]
#[serde(transparent)]
pub struct MaterialId(pub u16);

impl MaterialId {
    pub const AIR: Self = Self(0);

    /// Never a real material: in an edit overlay it means "keep the
    /// generated base here" (decision 0006). The material table refuses it.
    pub const KEEP: Self = Self(u16::MAX);

    pub const fn is_air(self) -> bool {
        self.0 == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_air() {
        assert_eq!(MaterialId::default(), MaterialId::AIR);
        assert!(MaterialId::AIR.is_air());
        assert!(!MaterialId(5).is_air());
    }
}
