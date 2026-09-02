use crate::Fin;

const MAX_RELATIONSHIPS: usize = 48;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipKind {
    DependsOn,
    Provides,
    Contains,
    ConfiguredBy,
    Revises,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Relationship {
    pub source: Fin,
    pub target: Fin,
    pub kind: RelationshipKind,
    pub dimension: Option<Fin>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RelationshipError {
    Invalid,
    Duplicate,
    Full,
}

pub struct RelationshipGraph {
    entries: [Option<Relationship>; MAX_RELATIONSHIPS],
}

impl RelationshipGraph {
    pub const fn new() -> Self {
        Self {
            entries: [None; MAX_RELATIONSHIPS],
        }
    }

    pub fn relate(&mut self, relationship: Relationship) -> Result<(), RelationshipError> {
        if relationship.source.is_zero()
            || relationship.target.is_zero()
            || relationship.source == relationship.target
        {
            return Err(RelationshipError::Invalid);
        }
        if self
            .entries
            .iter()
            .flatten()
            .any(|entry| *entry == relationship)
        {
            return Err(RelationshipError::Duplicate);
        }
        let slot = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_none())
            .ok_or(RelationshipError::Full)?;
        *slot = Some(relationship);
        Ok(())
    }

    pub fn remove(&mut self, relationship: Relationship) -> bool {
        if let Some(slot) = self
            .entries
            .iter_mut()
            .find(|slot| slot.is_some_and(|entry| entry == relationship))
        {
            *slot = None;
            true
        } else {
            false
        }
    }

    pub fn targets<'a>(
        &'a self,
        source: Fin,
        kind: RelationshipKind,
        dimension: Option<Fin>,
    ) -> impl Iterator<Item = Fin> + 'a {
        self.entries.iter().flatten().filter_map(move |entry| {
            (entry.source == source
                && entry.kind == kind
                && (entry.dimension.is_none() || entry.dimension == dimension))
                .then_some(entry.target)
        })
    }

    pub fn referenced(&self, target: Fin) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.target == target)
    }

    pub fn involves(&self, fin: Fin) -> bool {
        self.entries
            .iter()
            .flatten()
            .any(|entry| entry.source == fin || entry.target == fin)
    }

    pub fn count(&self) -> usize {
        self.entries.iter().flatten().count()
    }
}

impl Default for RelationshipGraph {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relationships_are_typed_and_dimension_aware() {
        let source = Fin::from_u128(1);
        let target = Fin::from_u128(2);
        let stable = Fin::from_u128(3);
        let mut graph = RelationshipGraph::new();
        graph
            .relate(Relationship {
                source,
                target,
                kind: RelationshipKind::DependsOn,
                dimension: Some(stable),
            })
            .unwrap();
        assert_eq!(
            graph
                .targets(source, RelationshipKind::DependsOn, Some(stable))
                .next(),
            Some(target)
        );
        assert_eq!(
            graph
                .targets(source, RelationshipKind::DependsOn, Some(Fin::from_u128(4)))
                .next(),
            None
        );
        assert!(graph.involves(source));
        assert_eq!(graph.count(), 1);
        assert!(graph.referenced(target));
    }
}
