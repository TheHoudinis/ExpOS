//! Explicit Form reachability confinement.
//!
//! A Dimension identifies the execution context in which this scope applies,
//! but it does not grant access. Reachability comes only from this bounded
//! allowlist; Dimension bindings and `security_boundary` metadata remain
//! separate concerns.

use crate::{
    cfc::{Cfc, CfcFin, MAX_CFC_FORMS},
    Fin,
};

pub const MAX_SCOPE_TARGETS: usize = 8;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScopeError {
    UnownedDimension,
    UnownedForm,
    Duplicate,
    Full,
    Denied,
}

/// The Forms one subject may reach while executing in a CFC/Dimension
/// context. All identity is stored by FIN and no allocation is required.
pub struct ExpScope {
    cfc: CfcFin,
    dimension: Fin,
    subject: Fin,
    /// Immutable ownership snapshot taken when the scope is created. Later
    /// callers cannot substitute another `Cfc` carrying the same numeric FIN
    /// to smuggle a foreign Form into an existing scope.
    owned_forms: [Option<Fin>; MAX_CFC_FORMS],
    targets: [Option<Fin>; MAX_SCOPE_TARGETS],
}

impl ExpScope {
    /// Create a deny-by-default scope for a CFC-owned subject and Dimension.
    ///
    /// The subject does not need a Dimension binding: a binding describes
    /// placement, not confinement authority.
    pub fn new(cfc: &Cfc, dimension: Fin, subject: Fin) -> Result<Self, ScopeError> {
        if !cfc.owns_dimension(dimension) {
            return Err(ScopeError::UnownedDimension);
        }
        if !cfc.owns_form(subject) {
            return Err(ScopeError::UnownedForm);
        }
        let mut owned_forms = [None; MAX_CFC_FORMS];
        for (index, slot) in owned_forms.iter_mut().enumerate() {
            *slot = cfc.form(index);
        }
        Ok(Self {
            cfc: cfc.fin(),
            dimension,
            subject,
            owned_forms,
            targets: [None; MAX_SCOPE_TARGETS],
        })
    }

    pub const fn cfc(&self) -> CfcFin {
        self.cfc
    }

    pub const fn dimension(&self) -> Fin {
        self.dimension
    }

    pub const fn subject(&self) -> Fin {
        self.subject
    }

    pub fn len(&self) -> usize {
        self.targets.iter().flatten().count()
    }

    pub fn is_empty(&self) -> bool {
        self.targets.iter().all(Option::is_none)
    }

    pub fn targets(&self) -> impl Iterator<Item = Fin> + '_ {
        self.targets.iter().flatten().copied()
    }

    pub fn contains(&self, target: Fin) -> bool {
        self.targets().any(|allowed| allowed == target)
    }

    /// Add one Form from the immutable CFC ownership snapshot to the reachable
    /// set. A newly constructed or mutated CFC cannot enlarge this scope.
    pub fn allow(&mut self, target: Fin) -> Result<(), ScopeError> {
        self.validate_target(target)?;
        if self.contains(target) {
            return Err(ScopeError::Duplicate);
        }
        let slot = self
            .targets
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(ScopeError::Full)?;
        *slot = Some(target);
        Ok(())
    }

    /// Validate a reachability request against both CFC ownership and the
    /// explicit allowlist. Absence is denial; there is no ambient access from
    /// sharing a Dimension.
    pub fn authorize(&self, target: Fin) -> Result<(), ScopeError> {
        self.validate_target(target)?;
        if self.contains(target) {
            Ok(())
        } else {
            Err(ScopeError::Denied)
        }
    }

    fn validate_target(&self, target: Fin) -> Result<(), ScopeError> {
        if !self
            .owned_forms
            .iter()
            .flatten()
            .any(|owned| *owned == target)
        {
            return Err(ScopeError::UnownedForm);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fin(value: u128) -> Fin {
        Fin::from_u128(value)
    }

    fn cfc(value: u128, dimension: Fin) -> Cfc {
        Cfc::new(CfcFin::from_u128(value), "Test CFC", dimension).unwrap()
    }

    #[test]
    fn construction_requires_cfc_owned_dimension_and_subject() {
        let primary = fin(10);
        let subject = fin(20);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();

        let scope = ExpScope::new(&owner, primary, subject).unwrap();
        assert_eq!(scope.cfc(), owner.fin());
        assert_eq!(scope.dimension(), primary);
        assert_eq!(scope.subject(), subject);
        assert!(scope.is_empty());

        assert!(matches!(
            ExpScope::new(&owner, fin(11), subject),
            Err(ScopeError::UnownedDimension)
        ));
        assert!(matches!(
            ExpScope::new(&owner, primary, fin(21)),
            Err(ScopeError::UnownedForm)
        ));
    }

    #[test]
    fn dimension_membership_does_not_implicitly_grant_reachability() {
        let primary = fin(10);
        let subject = fin(20);
        let target = fin(21);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();
        owner.own_form(target).unwrap();
        owner.bind(subject, primary).unwrap();
        owner.bind(target, primary).unwrap();

        let mut scope = ExpScope::new(&owner, primary, subject).unwrap();
        assert_eq!(scope.authorize(target), Err(ScopeError::Denied));
        scope.allow(target).unwrap();
        assert_eq!(scope.authorize(target), Ok(()));
    }

    #[test]
    fn an_unbound_owned_form_can_be_explicitly_allowed() {
        let primary = fin(10);
        let subject = fin(20);
        let target = fin(21);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();
        owner.own_form(target).unwrap();

        let mut scope = ExpScope::new(&owner, primary, subject).unwrap();
        scope.allow(target).unwrap();
        assert!(scope.contains(target));
    }

    #[test]
    fn cross_cfc_and_unowned_forms_are_rejected() {
        let primary = fin(10);
        let subject = fin(20);
        let target = fin(21);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();
        owner.own_form(target).unwrap();

        let foreign_primary = fin(30);
        let foreign_target = fin(31);
        let mut foreign = cfc(2, foreign_primary);
        foreign.own_form(foreign_target).unwrap();

        let mut scope = ExpScope::new(&owner, primary, subject).unwrap();
        assert_eq!(scope.allow(foreign_target), Err(ScopeError::UnownedForm));
        assert_eq!(
            scope.authorize(foreign_target),
            Err(ScopeError::UnownedForm)
        );
        assert!(scope.is_empty());
    }

    #[test]
    fn an_impostor_with_the_same_cfc_fin_cannot_expand_an_existing_scope() {
        let primary = fin(10);
        let subject = fin(20);
        let target = fin(21);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();
        owner.own_form(target).unwrap();
        let mut scope = ExpScope::new(&owner, primary, subject).unwrap();

        let mut impostor = cfc(1, fin(30));
        let foreign_target = fin(31);
        impostor.own_form(foreign_target).unwrap();
        assert_eq!(impostor.fin(), owner.fin());
        assert!(impostor.owns_form(foreign_target));
        assert_eq!(scope.allow(foreign_target), Err(ScopeError::UnownedForm));
        scope.allow(target).unwrap();
        assert_eq!(scope.authorize(target), Ok(()));
    }

    #[test]
    fn duplicate_and_full_scopes_are_rejected_without_mutation() {
        let primary = fin(10);
        let subject = fin(20);
        let mut owner = cfc(1, primary);
        owner.own_form(subject).unwrap();
        for offset in 0..=MAX_SCOPE_TARGETS {
            owner.own_form(fin(100 + offset as u128)).unwrap();
        }
        let mut scope = ExpScope::new(&owner, primary, subject).unwrap();
        for offset in 0..MAX_SCOPE_TARGETS {
            scope.allow(fin(100 + offset as u128)).unwrap();
        }
        assert_eq!(scope.len(), MAX_SCOPE_TARGETS);
        assert_eq!(scope.allow(fin(100)), Err(ScopeError::Duplicate));
        assert_eq!(
            scope.allow(fin(100 + MAX_SCOPE_TARGETS as u128)),
            Err(ScopeError::Full)
        );
        assert_eq!(scope.len(), MAX_SCOPE_TARGETS);
    }
}
