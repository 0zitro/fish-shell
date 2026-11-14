//! The enclosing scopes one character is inside.

use std::ops::{BitOr, BitOrAssign};

use crate::entries::{ENCLOSING_SCOPE_IDS, TokenScopeId};

/// The enclosing scopes one character is inside.
///
/// They are members of the character's scope set that the visitor's enclosing
/// constructs contribute, rather than ones the leaf's chain adds.  They are
/// entries like any other and each of them is colourable; what makes a scope
/// enclosing is its position in the set — after the chain, before the root.
///
/// A set rather than a stack: a scope that appears twice is not expressible,
/// because a set records membership and not a count, and it therefore records no
/// nesting either.  The scopes are iterated in path order — deepest path first,
/// then by path — so a refinement precedes its family, and the order the members
/// were declared in carries nothing.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub struct EnclosingScopeSet(u16);

impl EnclosingScopeSet {
    /// Whether the set contains this enclosing scope.
    ///
    /// A scope that encloses nothing is in no set: the declaration gives a bit
    /// only to the scopes marked enclosing.
    pub fn has_scope(self, scope: TokenScopeId) -> bool {
        scope
            .enclosing_bit()
            .is_some_and(|bit| self.0 & (1u16 << bit) != 0)
    }

    /// A set containing this scope in addition to what this one contains.
    pub fn with_scope(self, scope: TokenScopeId) -> Self {
        match scope.enclosing_bit() {
            Some(bit) => Self(self.0 | (1u16 << bit)),
            None => self,
        }
    }

    /// Whether the set contains no scope.
    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    /// The scopes of the set, in path order.
    pub fn scopes(self) -> impl Iterator<Item = TokenScopeId> {
        ENCLOSING_SCOPE_IDS
            .iter()
            .copied()
            .enumerate()
            .filter(move |(bit, _)| self.0 & (1u16 << bit) != 0)
            .map(|(_, scope)| scope)
    }
}

/// A set's union: the visitor computes `inherited | region`, and `Default` is the
/// empty set, which is the identity, so the laws hold.
impl BitOr for EnclosingScopeSet {
    type Output = Self;

    fn bitor(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl BitOrAssign for EnclosingScopeSet {
    fn bitor_assign(&mut self, other: Self) {
        self.0 |= other.0;
    }
}

/// A set holds one bit per declared enclosing scope.
const _: () = assert!(ENCLOSING_SCOPE_IDS.len() <= u16::BITS as usize);

#[cfg(test)]
mod tests {
    use super::EnclosingScopeSet;
    use crate::entries::TokenScopeId;

    #[test]
    fn the_set_iterates_its_members_deepest_first() {
        let set = EnclosingScopeSet::default()
            .with_scope(TokenScopeId::META_LOOP)
            .with_scope(TokenScopeId::META_LOOP_FOR);

        assert_eq!(
            set.scopes().collect::<Vec<_>>(),
            vec![TokenScopeId::META_LOOP_FOR, TokenScopeId::META_LOOP],
            "a refinement is consulted before its family"
        );
    }

    #[test]
    fn a_scope_that_encloses_nothing_is_in_no_set() {
        assert!(
            EnclosingScopeSet::default()
                .with_scope(TokenScopeId::COMMENT)
                .is_empty()
        );
    }

    #[test]
    fn scopes_that_differ_are_members_together() {
        let set = EnclosingScopeSet::default()
            .with_scope(TokenScopeId::META_FUNCTION)
            .with_scope(TokenScopeId::META_PIPELINE);

        assert!(set.has_scope(TokenScopeId::META_FUNCTION));
        assert!(set.has_scope(TokenScopeId::META_PIPELINE));
        assert!(!set.has_scope(TokenScopeId::META_BLOCK));
    }
}
