//! The closed vocabulary of token scopes.
//!
//! A token scope names one lexical class of source text: the class a character
//! belongs to, and the class a pager row segment belongs to.  The vocabulary is
//! closed: every scope is declared, and `declare_scope_entries!` expands that
//! declaration into the entry table, one [`TokenScopeId`] constant per scope, and
//! the chain each scope is resolved through.  Nothing here consults the parse
//! tree, the environment or a style.
//!
//! The declaration is the only place a scope, its path, its variable name, its
//! chain and its enclosing bit are stated.  A second statement of any of them —
//! a name helper, a fallback match, a role enum — is a second vocabulary.

mod attribute;
mod chain;
mod enclosing;
mod entries;

pub use attribute::{
    AttributeMode, HighlightAttribute, HighlightAttributeEntry, HighlightAttributeSet, StyleField,
    StyleFieldSet,
};
pub use chain::{ResolutionChainStep, ScopeResolutionChain};
pub use enclosing::EnclosingScopeSet;
pub use entries::{ColorVariableName, TOKEN_SCOPE_ENTRIES, TokenScopeEntry, TokenScopeId};
