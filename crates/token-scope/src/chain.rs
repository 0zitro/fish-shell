//! The order in which one character's scopes and attributes are consulted.

use crate::attribute::HighlightAttribute;
use crate::entries::TokenScopeId;

/// One step of a resolution chain: a scope's variable, or an attribute's.
///
/// The attribute form exists because one declared fallback crosses from the pager
/// namespace into the attribute table: `fish_pager_color_selected_background`
/// falls back to `fish_color_search_match`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum ResolutionChainStep {
    /// The step names a scope; the variable is the scope's.
    Scope(TokenScopeId),
    /// The step names an attribute; the variable is the attribute's.
    Attribute(HighlightAttribute),
}

/// The steps consulted for one character, most specific first, excluding the
/// character's own scope and ending at the root scope.
///
/// The root is implicit: every chain ends there, so the declaration does not
/// write it, and the root's own chain is empty.  An entry that writes no chain
/// has one derived from its path; an entry that writes a chain has exactly that
/// chain.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ScopeResolutionChain(pub(crate) &'static [ResolutionChainStep]);

/// A chain is an ordered sequence; walking it in order is the resolution.
impl IntoIterator for ScopeResolutionChain {
    type Item = ResolutionChainStep;
    type IntoIter = std::iter::Copied<std::slice::Iter<'static, ResolutionChainStep>>;

    fn into_iter(self) -> Self::IntoIter {
        self.0.iter().copied()
    }
}
