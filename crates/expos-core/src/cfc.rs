use core::fmt;

use crate::{Fin, Text};

/// A CFC can contain eight Dimensions in total: one immutable Primary
/// Dimension and up to seven secondary Dimensions.
pub const MAX_CFC_SECONDARY_DIMENSIONS: usize = 7;
pub const MAX_CFC_FORMS: usize = 16;
pub const MAX_CFC_BINDINGS: usize = 32;
pub const MAX_CFCS: usize = 4;

/// Stable identity for a Central Inflation Fabric.
///
/// This is intentionally not a [`Fin`]. A FIN identifies a Form (and is also
/// used by the existing Dimension model); a `CfcFin` identifies the containing
/// CFC. The two types cannot be passed in place of one another accidentally.
#[derive(Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct CfcFin([u8; 16]);

impl CfcFin {
    pub const ZERO: Self = Self([0; 16]);

    pub const fn from_u128(value: u128) -> Self {
        Self(value.to_be_bytes())
    }

    pub const fn bytes(self) -> [u8; 16] {
        self.0
    }

    pub const fn is_zero(self) -> bool {
        let mut index = 0;
        while index < self.0.len() {
            if self.0[index] != 0 {
                return false;
            }
            index += 1;
        }
        true
    }
}

impl fmt::Debug for CfcFin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl fmt::Display for CfcFin {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        for (index, byte) in self.0.iter().enumerate() {
            write!(formatter, "{byte:02X}")?;
            if matches!(index, 3 | 5 | 7 | 9) {
                formatter.write_str("-")?;
            }
        }
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct CfcBinding {
    pub form: Fin,
    pub dimension: Fin,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfcError {
    InvalidCfcFin,
    InvalidName,
    InvalidIdentity,
    DuplicateDimension,
    DuplicateForm,
    IdentityKindConflict,
    SecondaryDimensionCapacity,
    FormCapacity,
    BindingCapacity,
    MissingDimension,
    MissingForm,
    AlreadyBound,
}

/// One complete ExpOS environment and its locally owned identities.
///
/// The Primary Dimension is required by `new`, kept in a private field, and
/// has no replacement operation. Forms may be bound into more than one local
/// Dimension, but the catalog prevents either identity kind from being owned
/// by another CFC.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Cfc {
    fin: CfcFin,
    name: Text,
    primary_dimension: Fin,
    secondary_dimensions: [Option<Fin>; MAX_CFC_SECONDARY_DIMENSIONS],
    forms: [Option<Fin>; MAX_CFC_FORMS],
    bindings: [Option<CfcBinding>; MAX_CFC_BINDINGS],
}

impl Cfc {
    pub fn new(fin: CfcFin, name: &str, primary_dimension: Fin) -> Result<Self, CfcError> {
        if fin.is_zero() {
            return Err(CfcError::InvalidCfcFin);
        }
        if primary_dimension.is_zero() {
            return Err(CfcError::InvalidIdentity);
        }
        let name = Text::new(name.trim()).map_err(|_| CfcError::InvalidName)?;
        Ok(Self {
            fin,
            name,
            primary_dimension,
            secondary_dimensions: [None; MAX_CFC_SECONDARY_DIMENSIONS],
            forms: [None; MAX_CFC_FORMS],
            bindings: [None; MAX_CFC_BINDINGS],
        })
    }

    pub const fn fin(&self) -> CfcFin {
        self.fin
    }

    pub const fn name(&self) -> &Text {
        &self.name
    }

    pub const fn primary_dimension(&self) -> Fin {
        self.primary_dimension
    }

    pub fn dimension_count(&self) -> usize {
        1 + self.secondary_dimensions.iter().flatten().count()
    }

    pub fn form_count(&self) -> usize {
        self.forms.iter().flatten().count()
    }

    pub fn binding_count(&self) -> usize {
        self.bindings.iter().flatten().count()
    }

    pub fn secondary_dimension(&self, index: usize) -> Option<Fin> {
        self.secondary_dimensions
            .iter()
            .flatten()
            .nth(index)
            .copied()
    }

    pub fn form(&self, index: usize) -> Option<Fin> {
        self.forms.iter().flatten().nth(index).copied()
    }

    pub fn binding(&self, index: usize) -> Option<CfcBinding> {
        self.bindings.iter().flatten().nth(index).copied()
    }

    pub fn owns_dimension(&self, dimension: Fin) -> bool {
        dimension == self.primary_dimension
            || self
                .secondary_dimensions
                .iter()
                .flatten()
                .any(|candidate| *candidate == dimension)
    }

    pub fn owns_form(&self, form: Fin) -> bool {
        self.forms
            .iter()
            .flatten()
            .any(|candidate| *candidate == form)
    }

    pub fn is_bound(&self, form: Fin, dimension: Fin) -> bool {
        self.bindings
            .iter()
            .flatten()
            .any(|binding| binding.form == form && binding.dimension == dimension)
    }

    pub fn add_secondary_dimension(&mut self, dimension: Fin) -> Result<(), CfcError> {
        if dimension.is_zero() {
            return Err(CfcError::InvalidIdentity);
        }
        if self.owns_dimension(dimension) {
            return Err(CfcError::DuplicateDimension);
        }
        if self.owns_form(dimension) {
            return Err(CfcError::IdentityKindConflict);
        }
        let slot = self
            .secondary_dimensions
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CfcError::SecondaryDimensionCapacity)?;
        *slot = Some(dimension);
        Ok(())
    }

    pub fn own_form(&mut self, form: Fin) -> Result<(), CfcError> {
        if form.is_zero() {
            return Err(CfcError::InvalidIdentity);
        }
        if self.owns_form(form) {
            return Err(CfcError::DuplicateForm);
        }
        if self.owns_dimension(form) {
            return Err(CfcError::IdentityKindConflict);
        }
        let slot = self
            .forms
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CfcError::FormCapacity)?;
        *slot = Some(form);
        Ok(())
    }

    /// Bind an owned Form only to an owned Dimension in this CFC.
    pub fn bind(&mut self, form: Fin, dimension: Fin) -> Result<(), CfcError> {
        if !self.owns_form(form) {
            return Err(CfcError::MissingForm);
        }
        if !self.owns_dimension(dimension) {
            return Err(CfcError::MissingDimension);
        }
        if self.is_bound(form, dimension) {
            return Err(CfcError::AlreadyBound);
        }
        let slot = self
            .bindings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CfcError::BindingCapacity)?;
        *slot = Some(CfcBinding { form, dimension });
        Ok(())
    }

    /// Recheck every local ownership and binding invariant.
    ///
    /// `CfcCatalog::insert` invokes this before it publishes a CFC.
    pub fn validate(&self) -> Result<(), CfcError> {
        if self.fin.is_zero() {
            return Err(CfcError::InvalidCfcFin);
        }
        if self.name.as_str().trim().is_empty() {
            return Err(CfcError::InvalidName);
        }
        if self.primary_dimension.is_zero() {
            return Err(CfcError::InvalidIdentity);
        }
        for (index, dimension) in self.secondary_dimensions.iter().flatten().enumerate() {
            if dimension.is_zero() || *dimension == self.primary_dimension {
                return Err(CfcError::DuplicateDimension);
            }
            if self
                .secondary_dimensions
                .iter()
                .flatten()
                .skip(index + 1)
                .any(|candidate| candidate == dimension)
            {
                return Err(CfcError::DuplicateDimension);
            }
            if self.owns_form(*dimension) {
                return Err(CfcError::IdentityKindConflict);
            }
        }
        for (index, form) in self.forms.iter().flatten().enumerate() {
            if form.is_zero() {
                return Err(CfcError::InvalidIdentity);
            }
            if self
                .forms
                .iter()
                .flatten()
                .skip(index + 1)
                .any(|candidate| candidate == form)
            {
                return Err(CfcError::DuplicateForm);
            }
            if self.owns_dimension(*form) {
                return Err(CfcError::IdentityKindConflict);
            }
        }
        for (index, binding) in self.bindings.iter().flatten().enumerate() {
            if !self.owns_form(binding.form) {
                return Err(CfcError::MissingForm);
            }
            if !self.owns_dimension(binding.dimension) {
                return Err(CfcError::MissingDimension);
            }
            if self
                .bindings
                .iter()
                .flatten()
                .skip(index + 1)
                .any(|candidate| candidate == binding)
            {
                return Err(CfcError::AlreadyBound);
            }
        }
        Ok(())
    }

    fn owns_any_identity(&self, identity: Fin) -> bool {
        self.owns_dimension(identity) || self.owns_form(identity)
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CfcCatalogError {
    InvalidCfc(CfcError),
    DuplicateCfcFin,
    CrossCfcSharing {
        identity: Fin,
        existing_owner: CfcFin,
    },
    Full,
}

/// Fixed-capacity ownership catalog for machines with more than one CFC.
pub struct CfcCatalog {
    cfcs: [Option<Cfc>; MAX_CFCS],
}

impl CfcCatalog {
    pub const fn new() -> Self {
        Self {
            cfcs: [None; MAX_CFCS],
        }
    }

    /// Publish a complete CFC atomically after local and cross-CFC validation.
    pub fn insert(&mut self, cfc: Cfc) -> Result<(), CfcCatalogError> {
        cfc.validate().map_err(CfcCatalogError::InvalidCfc)?;
        if self.cfcs.iter().flatten().any(|entry| entry.fin == cfc.fin) {
            return Err(CfcCatalogError::DuplicateCfcFin);
        }
        for existing in self.cfcs.iter().flatten() {
            if existing.owns_any_identity(cfc.primary_dimension) {
                return Err(CfcCatalogError::CrossCfcSharing {
                    identity: cfc.primary_dimension,
                    existing_owner: existing.fin,
                });
            }
            for identity in cfc.secondary_dimensions.iter().flatten() {
                if existing.owns_any_identity(*identity) {
                    return Err(CfcCatalogError::CrossCfcSharing {
                        identity: *identity,
                        existing_owner: existing.fin,
                    });
                }
            }
            for identity in cfc.forms.iter().flatten() {
                if existing.owns_any_identity(*identity) {
                    return Err(CfcCatalogError::CrossCfcSharing {
                        identity: *identity,
                        existing_owner: existing.fin,
                    });
                }
            }
        }
        let slot = self
            .cfcs
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(CfcCatalogError::Full)?;
        *slot = Some(cfc);
        Ok(())
    }

    pub fn cfc(&self, fin: CfcFin) -> Option<&Cfc> {
        self.cfcs.iter().flatten().find(|cfc| cfc.fin == fin)
    }

    pub fn len(&self) -> usize {
        self.cfcs.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Default for CfcCatalog {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(value: u128) -> Fin {
        Fin::from_u128(value)
    }

    fn cfc_fin(value: u128) -> CfcFin {
        CfcFin::from_u128(value)
    }

    #[test]
    fn cfc_identity_is_a_distinct_stable_type_and_name_is_required() {
        let id = cfc_fin(0xCAFE);
        assert_eq!(id.bytes(), Fin::from_u128(0xCAFE).bytes());
        assert_eq!(core::mem::size_of::<CfcFin>(), 16);
        assert_eq!(
            Cfc::new(CfcFin::ZERO, "Machine", fin(1)),
            Err(CfcError::InvalidCfcFin)
        );
        assert_eq!(Cfc::new(id, "   ", fin(1)), Err(CfcError::InvalidName));
        assert_eq!(
            Cfc::new(id, "Machine", Fin::ZERO),
            Err(CfcError::InvalidIdentity)
        );
        let cfc = Cfc::new(id, "  Workstation  ", fin(1)).unwrap();
        assert_eq!(cfc.name().as_str(), "Workstation");
    }

    #[test]
    fn one_primary_is_mandatory_immutable_and_not_a_secondary() {
        let mut cfc = Cfc::new(cfc_fin(1), "Machine", fin(10)).unwrap();
        assert_eq!(cfc.primary_dimension(), fin(10));
        assert_eq!(cfc.dimension_count(), 1);
        assert_eq!(
            cfc.add_secondary_dimension(fin(10)),
            Err(CfcError::DuplicateDimension)
        );
        cfc.add_secondary_dimension(fin(11)).unwrap();
        assert_eq!(cfc.primary_dimension(), fin(10));
        assert_eq!(cfc.dimension_count(), 2);
    }

    #[test]
    fn local_ownership_and_binding_are_bounded_and_validated() {
        let mut cfc = Cfc::new(cfc_fin(1), "Machine", fin(10)).unwrap();
        assert_eq!(cfc.bind(fin(100), fin(10)), Err(CfcError::MissingForm));
        cfc.own_form(fin(100)).unwrap();
        assert_eq!(cfc.bind(fin(100), fin(99)), Err(CfcError::MissingDimension));
        cfc.bind(fin(100), fin(10)).unwrap();
        assert_eq!(cfc.bind(fin(100), fin(10)), Err(CfcError::AlreadyBound));
        cfc.add_secondary_dimension(fin(11)).unwrap();
        cfc.bind(fin(100), fin(11)).unwrap();
        assert!(cfc.is_bound(fin(100), fin(10)));
        assert!(cfc.is_bound(fin(100), fin(11)));
        assert_eq!(cfc.validate(), Ok(()));

        for value in 12..(12 + MAX_CFC_SECONDARY_DIMENSIONS as u128 - 1) {
            cfc.add_secondary_dimension(fin(value)).unwrap();
        }
        assert_eq!(cfc.dimension_count(), 1 + MAX_CFC_SECONDARY_DIMENSIONS);
        assert_eq!(
            cfc.add_secondary_dimension(fin(999)),
            Err(CfcError::SecondaryDimensionCapacity)
        );

        for value in 101..(101 + MAX_CFC_FORMS as u128 - 1) {
            cfc.own_form(fin(value)).unwrap();
        }
        assert_eq!(cfc.form_count(), MAX_CFC_FORMS);
        assert_eq!(cfc.own_form(fin(2_000)), Err(CfcError::FormCapacity));
    }

    #[test]
    fn a_fin_cannot_change_identity_kind_inside_a_cfc() {
        let mut cfc = Cfc::new(cfc_fin(1), "Machine", fin(10)).unwrap();
        assert_eq!(cfc.own_form(fin(10)), Err(CfcError::IdentityKindConflict));
        cfc.own_form(fin(100)).unwrap();
        assert_eq!(
            cfc.add_secondary_dimension(fin(100)),
            Err(CfcError::IdentityKindConflict)
        );
    }

    #[test]
    fn catalog_rejects_same_kind_and_cross_kind_sharing_atomically() {
        let mut first = Cfc::new(cfc_fin(1), "First", fin(10)).unwrap();
        first.add_secondary_dimension(fin(11)).unwrap();
        first.own_form(fin(100)).unwrap();

        let mut catalog = CfcCatalog::new();
        catalog.insert(first).unwrap();

        let second = Cfc::new(cfc_fin(2), "Second", fin(11)).unwrap();
        assert_eq!(
            catalog.insert(second),
            Err(CfcCatalogError::CrossCfcSharing {
                identity: fin(11),
                existing_owner: cfc_fin(1),
            })
        );
        assert_eq!(catalog.len(), 1);

        let cross_kind = Cfc::new(cfc_fin(3), "Cross kind", fin(100)).unwrap();
        assert_eq!(
            catalog.insert(cross_kind),
            Err(CfcCatalogError::CrossCfcSharing {
                identity: fin(100),
                existing_owner: cfc_fin(1),
            })
        );
        assert_eq!(catalog.len(), 1);

        let mut third = Cfc::new(cfc_fin(4), "Third", fin(20)).unwrap();
        third.own_form(fin(200)).unwrap();
        catalog.insert(third).unwrap();
        assert_eq!(catalog.len(), 2);
        assert!(catalog.cfc(cfc_fin(4)).unwrap().owns_form(fin(200)));
    }

    #[test]
    fn duplicate_cfc_identity_is_rejected_without_replacing_the_owner() {
        let mut catalog = CfcCatalog::new();
        catalog
            .insert(Cfc::new(cfc_fin(1), "Original", fin(10)).unwrap())
            .unwrap();
        assert_eq!(
            catalog.insert(Cfc::new(cfc_fin(1), "Replacement", fin(20)).unwrap()),
            Err(CfcCatalogError::DuplicateCfcFin)
        );
        assert_eq!(catalog.cfc(cfc_fin(1)).unwrap().name().as_str(), "Original");
    }
}
