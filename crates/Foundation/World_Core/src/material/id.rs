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
