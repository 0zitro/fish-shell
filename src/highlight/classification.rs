//! The classification of one character.

use fish_token_scope::{
    EnclosingScopeSet, HighlightAttribute, HighlightAttributeSet, TokenScopeId,
};

/// The classification of one character.
///
/// A character's scopes for one color channel are an ordered set: the scope in the
/// channel's slot, the scopes its chain adds, the enclosing scopes it is inside, and
/// the root.  Resolution composes their variables field by field, most specific last.
/// Two slots are carried because a pager row takes its background from one variable
/// and its segment's foreground from another.  A character of source text leaves the
/// background at the root scope, so the visitor's output uses the foreground slot
/// alone.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct HighlightClassification {
    foreground: TokenScopeId,
    background: TokenScopeId,
    enclosing: EnclosingScopeSet,
    attributes: HighlightAttributeSet,
}

impl HighlightClassification {
    /// A classification whose two colors come from different scopes, as a pager row
    /// is.
    pub fn of_scopes(foreground: TokenScopeId, background: TokenScopeId) -> Self {
        Self {
            foreground,
            background,
            enclosing: EnclosingScopeSet::default(),
            attributes: HighlightAttributeSet::default(),
        }
    }

    /// The scope that colors the foreground.
    pub fn foreground(self) -> TokenScopeId {
        self.foreground
    }

    /// The scope that colors the background.
    pub fn background(self) -> TokenScopeId {
        self.background
    }

    /// The enclosing scopes this character is inside.
    pub fn enclosing(self) -> EnclosingScopeSet {
        self.enclosing
    }

    /// The attributes layered on this character.
    pub fn attributes(self) -> HighlightAttributeSet {
        self.attributes
    }

    /// This classification, additionally inside these enclosing scopes.
    pub fn with_enclosing(self, enclosing: EnclosingScopeSet) -> Self {
        Self { enclosing, ..self }
    }

    /// This classification, with one more attribute.
    pub fn with_attribute(self, attribute: HighlightAttribute) -> Self {
        Self {
            attributes: self.attributes.with_attribute(attribute),
            ..self
        }
    }
}

/// An infallible widening: every scope denotes one classification, whose background
/// is the root.
impl From<TokenScopeId> for HighlightClassification {
    fn from(scope: TokenScopeId) -> Self {
        Self::of_scopes(scope, TokenScopeId::NORMAL)
    }
}

impl Default for HighlightClassification {
    /// The root scope, with no attributes: the classification of a character that no
    /// rule matches.
    fn default() -> Self {
        Self::from(TokenScopeId::NORMAL)
    }
}
