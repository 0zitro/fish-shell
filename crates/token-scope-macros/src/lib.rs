//! The declaration of the token-scope vocabulary.
//!
//! The vocabulary is closed: every scope a character can be classified as, the
//! variable that colours it, and the chain it is resolved through come from one
//! invocation of [`declare_scope_entries!`].  The invocation is the table; this
//! crate only reads it.
//!
//! The macro exists because a stem is a projection of a path — the path with `.`
//! as `_` — and no declarative macro can spell a derived identifier or a derived
//! string literal.  Writing the stem beside the path would be the second
//! statement of one fact the design forbids, so the derivation is performed here
//! and the expansion is one table.

mod declaration;
mod expansion;

/// Declares the closed vocabulary.
///
/// Grammar.  Every form is positional; a scope's stem is the path with `.` as
/// `_` unless it is written, and its chain is derived from the path unless it is
/// written.
///
/// ```text
/// declare_scope_entries! {
///
///     namespace "<variable prefix>" { form* }
///
///     form :=
///         | "<path>" <description> [<stem>] [<steps>] [root] [enclosing] ;
///         | sum "<family path>" [<stem>] { <declared> ... } ;
///         | product "<path>" [<steps>] <description> { sum { <declared> ... } ... } ;
///
///     <stem>        := <identifier>
///     <steps>       := [ <step> , ... ]      <step> := "<path>" | @<Attribute>
///     <declared>    := <identifier> <description>
///     <description> := the doc comment after the name, in Markdown
/// }
/// ```
///
/// **A description says what a scope is.**  It is the doc comment that follows the
/// path, written in Markdown so that the same words are the generated
/// documentation's line and the constant's own documentation.  Every scope states
/// one: a sum's variants and a product's factor values each state their own, and a
/// product's leaves compose their family's description with the values they take.
///
/// **The namespace names the variable.**  Every scope is declared inside a
/// namespace, and its variable is that namespace's prefix followed by its stem.
/// There is no other place to declare a scope, so every scope is colourable,
/// including the scopes an enclosing construct contributes: an enclosing scope
/// is an entry like any other, and its position in a character's set is what
/// makes it enclosing, not its declaration.
///
/// **A chain is derived from the path unless it is written.**  An entry that
/// writes no chain resolves through the nearest declared entry prefix of its
/// path, that prefix's own chain, and then the root.  An entry that writes a
/// chain resolves through exactly that chain, and the derivation does not apply:
/// the declaration states the chain, so no part of it means "skip the parent".
///
/// **A step is a path or an attribute.**  A quoted step names a scope by path; a
/// step written `@<Attribute>` names one of `HighlightAttribute`'s variants.
/// The one such step is the pager's selected-row background, whose fallback is
/// `fish_color_search_match` — an overlay the reader adds to the characters that
/// match the search, not a lexical class of source text.
///
/// **A path is the only spelling that decomposes**: `x_y_z` cannot say whether
/// it is `x.y_z` or `x_y.z`, while `x.y_z` states both the path and, by
/// flattening dots to underscores, the stem.
///
/// **The derived stem** is the path with `.` as `_`: `variable.parameter.argv`
/// is `variable_parameter_argv`.  A stem is written as an identifier, and only
/// where the variable name the configuration fixed is not that projection —
/// `source` is `normal`, `terminator` is `end`, `argument` is `param` — or where
/// a scope's chain leaves its family, as `string.escape.invalid`'s
/// `escape_invalid` does.
///
/// **The root is marked once.**  Every chain ends at the root, and the entry
/// marked `root` is it.  Its own chain is empty: the root is the last step of
/// every chain, its own included.
///
/// **A sum** enumerates names.  It declares the variants of the scope at the
/// family path: a variant's path is the family path and the variant joined by
/// `.`, its stem is the family stem and the variant joined by `_`, and its chain
/// derives from the family path like any entry's.  The stem prefix overrides the
/// family stem for a family whose variants are named for a part of the path
/// rather than its whole: `keyword.modifier`'s variants are `keyword_and`, not
/// `keyword_modifier_and`.///
/// **A product** is a product of sums, and each of its factors is a sum with no
/// head: a factor enumerates the values one name component of a leaf may take,
/// and contributes one component to every leaf, so it announces no path of its
/// own.  Every factor has an implicit absent variant, exclusive with its values;
/// a leaf that takes the absent variant on every factor is the family root.  The
/// components follow the factor order, and the chain generalises from the tail of
/// that order: a leaf drops its last component first, so it reaches the family
/// root before the steps the family writes.
///
/// **An enclosing scope is marked.**  The scopes an enclosing construct
/// contributes are marked `enclosing`; they are the members an
/// `EnclosingScopeSet` holds, and a member's bit is its position in the order
/// they are consulted in: deepest path first, then by path.
///
/// **The order of the declarations carries nothing.**  Ids are internal, a chain
/// is derived from the path, a product's paths from its factor lists, the order
/// the enclosing scopes are consulted in from their paths, and the application
/// order of the attributes from their declared values.  Nothing is read off the
/// position of one declaration among its siblings.
///
/// The names it writes are the constant names: one per scope, its stem in
/// screaming case.
#[proc_macro]
pub fn declare_scope_entries(input: proc_macro::TokenStream) -> proc_macro::TokenStream {
    expansion::expanded(input.into()).into()
}
