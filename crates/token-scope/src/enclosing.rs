//! The enclosing scopes one character is inside.

use std::fmt;

use crate::entries::{ENCLOSING_SCOPE_IDS, TokenScopeId};

/// The enclosing scopes one character is inside, outermost first.
///
/// They are members of the character's scope set that the visitor's enclosing
/// constructs contribute, rather than ones the leaf's chain adds.  They are entries like
/// any other and each of them is colourable; what makes a scope enclosing is its
/// position in the set — after the chain, before the root.
///
/// The set states the order the constructs that contribute these scopes are entered in,
/// because that order is the order they are consulted in: a construct entered inside
/// another stands after it, so its variable is the more specific of the two.  A scope
/// contributed again moves to the end for the same reason — the construct entered last
/// encloses the character most closely — so the set names a scope once and states the
/// nesting beside it.
#[derive(Clone, Copy, Default, PartialEq, Eq, Hash)]
pub struct EnclosingScopeSet([u8; Self::LEVELS]);

impl EnclosingScopeSet {
    /// How many constructs one character's set can name.
    ///
    /// A set names each declared enclosing scope once, so this bounds the declaration's
    /// enclosing scopes as much as it bounds a character's nesting: a character nested
    /// deeper than this loses the outermost construct's scope, and the declaration is
    /// refused at build time when it marks more scopes than a set holds.
    const LEVELS: usize = 20;

    /// Whether the set contains this enclosing scope.
    ///
    /// A scope that encloses nothing is in no set: the declaration gives an index only
    /// to the scopes marked enclosing.
    pub fn has_scope(self, scope: TokenScopeId) -> bool {
        self.scopes().any(|member| member == scope)
    }

    /// The set additionally inside a construct that contributes this scope.
    ///
    /// The construct is the innermost so far, so its scope takes the last position; a
    /// scope the set already names moves there rather than being named twice.
    pub fn with_scope(self, scope: TokenScopeId) -> Self {
        let Some(index) = scope.enclosing_index() else {
            return self;
        };
        let contributed = index + 1;
        let mut levels = [0u8; Self::LEVELS];
        let mut count = 0;
        for level in self.0 {
            if level == 0 || level == contributed {
                continue;
            }
            levels[count] = level;
            count += 1;
        }
        if count == Self::LEVELS {
            // The set is as deep as it states: the outermost construct loses its place.
            levels.copy_within(1.., 0);
            count -= 1;
        }
        levels[count] = contributed;
        Self(levels)
    }

    /// The set additionally inside every scope of `region`, which is entered after what
    /// this set states.
    ///
    /// A region states the scopes of one construct in the order the construct declares
    /// them, so a refinement of a family stands after the family it refines.
    pub fn entering(self, region: Self) -> Self {
        region.scopes().fold(self, Self::with_scope)
    }

    /// Whether the set contains no scope.
    pub fn is_empty(self) -> bool {
        self.0[0] == 0
    }

    /// The scopes of the set, outermost first: the order they are consulted in.
    pub fn scopes(self) -> impl Iterator<Item = TokenScopeId> {
        self.0
            .into_iter()
            .take_while(|level| *level != 0)
            .map(|level| ENCLOSING_SCOPE_IDS[usize::from(level - 1)])
    }
}

/// The scopes the set names, outermost first, each by its path.
impl fmt::Debug for EnclosingScopeSet {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.debug_set().entries(self.scopes()).finish()
    }
}

/// A set holds one level per declared enclosing scope.
const _: () = assert!(
    ENCLOSING_SCOPE_IDS.len() <= EnclosingScopeSet::LEVELS,
    "more scopes enclose than an `EnclosingScopeSet` has levels"
);

#[cfg(test)]
mod tests {
    use super::EnclosingScopeSet;
    use crate::entries::TokenScopeId;

    #[test]
    fn the_set_iterates_its_members_in_the_order_they_are_entered() {
        let set = EnclosingScopeSet::default()
            .with_scope(TokenScopeId::META_BLOCK)
            .with_scope(TokenScopeId::META_IF);

        assert_eq!(
            set.scopes().collect::<Vec<_>>(),
            vec![TokenScopeId::META_BLOCK, TokenScopeId::META_IF],
            "the construct entered inside the other is consulted after it"
        );
    }

    #[test]
    fn a_scope_entered_again_moves_to_the_end() {
        let set = EnclosingScopeSet::default()
            .with_scope(TokenScopeId::META_BLOCK)
            .with_scope(TokenScopeId::META_IF)
            .with_scope(TokenScopeId::META_BLOCK);

        assert_eq!(
            set.scopes().collect::<Vec<_>>(),
            vec![TokenScopeId::META_IF, TokenScopeId::META_BLOCK],
            "a block inside an if encloses the character more closely than the if does"
        );
        assert!(set.has_scope(TokenScopeId::META_BLOCK));
        assert!(!set.has_scope(TokenScopeId::META_FUNCTION));
    }

    #[test]
    fn a_region_is_entered_inside_what_encloses_it() {
        let outer = EnclosingScopeSet::default().with_scope(TokenScopeId::META_FUNCTION);
        let region = EnclosingScopeSet::default()
            .with_scope(TokenScopeId::META_LOOP)
            .with_scope(TokenScopeId::META_LOOP_FOR);

        assert_eq!(
            outer.entering(region).scopes().collect::<Vec<_>>(),
            vec![
                TokenScopeId::META_FUNCTION,
                TokenScopeId::META_LOOP,
                TokenScopeId::META_LOOP_FOR,
            ],
            "a region's scopes follow the scopes that enclose the region"
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
