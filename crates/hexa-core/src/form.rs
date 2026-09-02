use crate::{Fin, Text};

const MAX_FORMS: usize = 16;
const MAX_DIMENSIONS: usize = 8;
const MAX_BINDINGS: usize = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FormKind {
    Root,
    Service,
    Interface,
    Package,
    Driver,
    Data,
    Policy,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Lifecycle {
    Active,
    Retired,
    Recoverable,
    Removed,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Form {
    pub fin: Fin,
    pub name: Text,
    pub kind: FormKind,
    pub revision: u32,
    pub lifecycle: Lifecycle,
}

impl Form {
    pub fn new(fin: Fin, name: &str, kind: FormKind) -> Self {
        assert!(!fin.is_zero(), "a Form must have a non-zero FIN");
        Self {
            fin,
            name: Text::new(name).expect("invalid Form name"),
            kind,
            revision: 1,
            lifecycle: Lifecycle::Active,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Dimension {
    pub fin: Fin,
    pub name: Text,
    pub security_boundary: bool,
}

impl Dimension {
    pub fn new(fin: Fin, name: &str, security_boundary: bool) -> Self {
        assert!(!fin.is_zero(), "a Dimension must have a non-zero FIN");
        Self {
            fin,
            name: Text::new(name).expect("invalid Dimension name"),
            security_boundary,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Visible,
    Hidden,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Binding {
    pub form: Fin,
    pub dimension: Fin,
    pub active_revision: u32,
    pub visibility: Visibility,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RegistryError {
    DuplicateFin,
    Full,
    MissingForm,
    MissingDimension,
    AlreadyBound,
    StillBound,
}

pub struct FormRegistry {
    forms: [Option<Form>; MAX_FORMS],
    dimensions: [Option<Dimension>; MAX_DIMENSIONS],
    bindings: [Option<Binding>; MAX_BINDINGS],
}

impl FormRegistry {
    pub const fn new() -> Self {
        Self {
            forms: [None; MAX_FORMS],
            dimensions: [None; MAX_DIMENSIONS],
            bindings: [None; MAX_BINDINGS],
        }
    }

    pub fn register(&mut self, form: Form) -> Result<(), RegistryError> {
        if self.form(form.fin).is_some() || self.dimension(form.fin).is_some() {
            return Err(RegistryError::DuplicateFin);
        }
        let slot = self
            .forms
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistryError::Full)?;
        *slot = Some(form);
        Ok(())
    }

    pub fn add_dimension(&mut self, dimension: Dimension) -> Result<(), RegistryError> {
        if self.form(dimension.fin).is_some() || self.dimension(dimension.fin).is_some() {
            return Err(RegistryError::DuplicateFin);
        }
        let slot = self
            .dimensions
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistryError::Full)?;
        *slot = Some(dimension);
        Ok(())
    }

    pub fn form(&self, fin: Fin) -> Option<&Form> {
        self.forms.iter().flatten().find(|form| form.fin == fin)
    }

    pub fn dimension(&self, fin: Fin) -> Option<&Dimension> {
        self.dimensions
            .iter()
            .flatten()
            .find(|dimension| dimension.fin == fin)
    }

    pub fn first_named_in(&self, name: &str, dimension: Fin) -> Option<&Form> {
        self.bindings.iter().flatten().find_map(|binding| {
            if binding.dimension != dimension || binding.visibility == Visibility::Hidden {
                return None;
            }
            self.form(binding.form)
                .filter(|form| form.name.as_str() == name)
        })
    }

    pub fn bind(
        &mut self,
        form: Fin,
        dimension: Fin,
        active_revision: u32,
        visibility: Visibility,
    ) -> Result<(), RegistryError> {
        if self.form(form).is_none() {
            return Err(RegistryError::MissingForm);
        }
        if self.dimension(dimension).is_none() {
            return Err(RegistryError::MissingDimension);
        }
        if self
            .bindings
            .iter()
            .flatten()
            .any(|binding| binding.form == form && binding.dimension == dimension)
        {
            return Err(RegistryError::AlreadyBound);
        }
        let slot = self
            .bindings
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RegistryError::Full)?;
        *slot = Some(Binding {
            form,
            dimension,
            active_revision,
            visibility,
        });
        Ok(())
    }

    pub fn retire_from(&mut self, form: Fin, dimension: Fin) -> Result<(), RegistryError> {
        let binding = self
            .bindings
            .iter_mut()
            .flatten()
            .find(|binding| binding.form == form && binding.dimension == dimension)
            .ok_or(RegistryError::MissingForm)?;
        binding.visibility = Visibility::Hidden;
        if !self
            .bindings
            .iter()
            .flatten()
            .any(|binding| binding.form == form && binding.visibility == Visibility::Visible)
        {
            if let Some(record) = self
                .forms
                .iter_mut()
                .flatten()
                .find(|record| record.fin == form)
            {
                record.lifecycle = Lifecycle::Retired;
            }
        }
        Ok(())
    }

    pub fn reclaim(&mut self, form: Fin) -> Result<(), RegistryError> {
        if self
            .bindings
            .iter()
            .flatten()
            .any(|binding| binding.form == form && binding.visibility == Visibility::Visible)
        {
            return Err(RegistryError::StillBound);
        }
        let record = self
            .forms
            .iter_mut()
            .flatten()
            .find(|record| record.fin == form)
            .ok_or(RegistryError::MissingForm)?;
        record.lifecycle = Lifecycle::Removed;
        Ok(())
    }
}

impl Default for FormRegistry {
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

    #[test]
    fn duplicate_names_keep_distinct_identity() {
        let mut registry = FormRegistry::new();
        registry
            .register(Form::new(fin(1), "Browser", FormKind::Interface))
            .unwrap();
        registry
            .register(Form::new(fin(2), "Browser", FormKind::Interface))
            .unwrap();
        assert_ne!(
            registry.form(fin(1)).unwrap().fin,
            registry.form(fin(2)).unwrap().fin
        );
    }

    #[test]
    fn retirement_is_dimension_aware() {
        let mut registry = FormRegistry::new();
        registry
            .register(Form::new(fin(1), "Browser", FormKind::Interface))
            .unwrap();
        registry
            .add_dimension(Dimension::new(fin(2), "Stable", true))
            .unwrap();
        registry
            .add_dimension(Dimension::new(fin(3), "Dev", false))
            .unwrap();
        registry
            .bind(fin(1), fin(2), 4, Visibility::Visible)
            .unwrap();
        registry
            .bind(fin(1), fin(3), 5, Visibility::Visible)
            .unwrap();
        registry.retire_from(fin(1), fin(2)).unwrap();
        assert_eq!(registry.form(fin(1)).unwrap().lifecycle, Lifecycle::Active);
        registry.retire_from(fin(1), fin(3)).unwrap();
        assert_eq!(registry.form(fin(1)).unwrap().lifecycle, Lifecycle::Retired);
        registry.reclaim(fin(1)).unwrap();
        assert_eq!(registry.form(fin(1)).unwrap().lifecycle, Lifecycle::Removed);
    }
}
