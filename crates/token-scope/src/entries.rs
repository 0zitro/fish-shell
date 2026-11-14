//! The closed vocabulary: the ids, the variable names, the entry table, and the
//! declaration that generates the table.

use std::fmt;

use crate::chain::ScopeResolutionChain;
use fish_token_scope_macros::declare_scope_entries;
use fish_widestring::wstr;

/// The identity of one declared scope.
///
/// An id is meaningful only against [`TOKEN_SCOPE_ENTRIES`]; it is small,
/// copyable and hashable so that a classification can be a cache key.  The field
/// is private and the declaration is the only place a value is made, so an id
/// always names an entry.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
#[repr(transparent)]
pub struct TokenScopeId(u16);

impl TokenScopeId {
    /// The entry this id names.
    pub fn entry(self) -> &'static TokenScopeEntry {
        &TOKEN_SCOPE_ENTRIES[usize::from(self.0)]
    }
}

/// The dotted path of a scope is the id's user-facing text.
impl fmt::Display for TokenScopeId {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.entry().path)
    }
}

/// The name of the environment variable that colors a scope or an attribute.
///
/// The name is a compile-time literal: `fish_color_quote_single`,
/// `fish_pager_color_selected_prefix`, `fish_color_valid_path`.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct ColorVariableName(pub(crate) &'static wstr);

/// A lossless, allocation-free view: the name the environment is asked for.
impl AsRef<wstr> for ColorVariableName {
    fn as_ref(&self) -> &wstr {
        self.0
    }
}

/// One declared scope.
#[derive(Clone, Copy, Debug)]
pub struct TokenScopeEntry {
    /// The entry's scope.
    pub scope: TokenScopeId,
    /// The dotted path, root-first: `string.quoted.single`.
    pub path: &'static str,
    /// The variable that colors it.  Every entry has one: an entry is declared
    /// inside a namespace, and the namespace supplies the prefix.
    pub variable_name: ColorVariableName,
    /// The steps consulted for a character classified as this scope, most
    /// specific first, excluding this scope itself and ending at the root.
    pub resolution_chain: ScopeResolutionChain,
}

declare_scope_entries! {
    namespace "fish_color_" {
        "source" normal root;
        "comment";

        "keyword" ["command"];
        sum "keyword" { begin builtin case command else end exec for function if in switch while };
        "keyword.modifier" ["operator"];
        sum "keyword.modifier" keyword { and or not exclam time };

        "command";
        "command.external";
        "command.builtin" builtin;
        "command.function" function;
        "command.abbreviation";
        "command.implicit_cd";

        "argument" param;
        "argument.option" option;
        sum "argument.option" { short long gnu };
        "argument.option.numeric" option_numeric ["argument"];
        product "variable.definition" ["argument"] {
            sum { local function global universal }
            sum { exported unexported }
            sum { path non_path }
        };

        "string.quoted" quote;
        sum "string.quoted" { single double };
        sum "string.quoted.single" { delimiter };
        sum "string.quoted.double" { delimiter };
        "string.escape" escape;
        sum "string.escape" { unicode octal control };
        "string.escape.invalid" escape_invalid ["invalid"];

        "variable" ["operator"];
        "variable.sigil";
        "variable.sigil.dereference";
        "variable.slice";
        "variable.parameter";
        "variable.parameter.argv";
        "variable.parameter.named";

        "operator";
        "operator.glob";
        "operator.brace";
        "operator.parenthesis";
        "operator.tilde";
        "operator.dollar";
        "operator.logical";
        "operator.assignment";

        "redirection";
        sum "redirection" { input output append fd };

        "terminator" end;
        sum "terminator" { pipe background semicolon newline };

        "invalid" error;
        sum "invalid" { command path redirection };

        // The scopes an enclosing construct contributes.  They are entries like the
        // rest; a character's set places them after its leaf's chain and before the
        // root, and what makes one enclosing is that position, not its declaration.
        "meta.function.no_shadowing" enclosing;
        "meta.function" enclosing;
        "meta.command_substitution" enclosing;
        "meta.block" enclosing;
        "meta.if" enclosing;
        "meta.case" enclosing;
        "meta.loop.for" enclosing;
        "meta.loop.while" enclosing;
        "meta.loop" enclosing;
        "meta.pipeline" enclosing;
        "meta.and_or" enclosing;
    }

    namespace "fish_pager_color_" {
        "progress";
        "background";
        "prefix";
        "completion";
        "description";
        "secondary.background"  ["background"];
        "secondary.prefix"      ["prefix"];
        "secondary.completion"  ["completion"];
        "secondary.description" ["description"];
        "selected.background"   ["background", @SearchMatch];
        "selected.prefix"       ["prefix"];
        "selected.completion"   ["completion"];
        "selected.description"  ["description"];
    }
}

#[cfg(test)]
mod tests {
    use super::{TOKEN_SCOPE_ENTRIES, TokenScopeId};
    use crate::chain::ResolutionChainStep;

    #[test]
    fn an_id_names_its_entry() {
        for entry in TOKEN_SCOPE_ENTRIES {
            assert_eq!(
                entry.scope.entry().path,
                entry.path,
                "the id `{}` does not name its own entry",
                entry.path
            );
        }
    }

    #[test]
    fn every_chain_ends_at_the_root() {
        let root = TokenScopeId::NORMAL;

        for entry in TOKEN_SCOPE_ENTRIES {
            let steps = entry.resolution_chain.into_iter().collect::<Vec<_>>();

            if entry.scope == root {
                assert!(
                    steps.is_empty(),
                    "the root is the last step of every chain, its own included"
                );
                continue;
            }

            assert_eq!(
                steps.last(),
                Some(&ResolutionChainStep::Scope(root)),
                "the chain of `{}` does not reach the root",
                entry.path
            );
            assert!(
                !steps.contains(&ResolutionChainStep::Scope(entry.scope)),
                "the chain of `{}` contains the scope itself",
                entry.path
            );

            let mut unique = steps.clone();
            unique.sort_by_key(|step| format!("{step:?}"));
            unique.dedup();
            assert_eq!(
                unique.len(),
                steps.len(),
                "the chain of `{}` repeats a step",
                entry.path
            );
        }
    }

    #[test]
    fn a_refinement_resolves_through_its_family() {
        /// The chain of one scope, as the paths it consults and the attributes it
        /// names.
        fn chain_of(path: &str) -> Vec<String> {
            let entry = TOKEN_SCOPE_ENTRIES
                .iter()
                .find(|entry| entry.path == path)
                .expect("the scope is declared");

            entry
                .resolution_chain
                .into_iter()
                .map(|step| match step {
                    ResolutionChainStep::Scope(scope) => scope.to_string(),
                    ResolutionChainStep::Attribute(attribute) => format!("@{attribute:?}"),
                })
                .collect()
        }

        let cases: &[(&str, &[&str])] = &[
            // A quote's marks resolve through the quote style and the family.
            (
                "string.quoted.double.delimiter",
                &["string.quoted.double", "string.quoted", "source"],
            ),
            // A written chain replaces the derivation: the modifier family falls
            // back to the operator, and a modifier to its family.
            (
                "keyword.modifier.and",
                &["keyword.modifier", "operator", "source"],
            ),
            // A path derivation reaches the family of every enclosing step.
            (
                "variable.sigil.dereference",
                &["variable.sigil", "variable", "operator", "source"],
            ),
            // A product's leaf generalises from the tail of the factor list.
            (
                "variable.definition.local.exported.path",
                &[
                    "variable.definition.local.exported",
                    "variable.definition.local",
                    "variable.definition",
                    "argument",
                    "source",
                ],
            ),
            // The numeric shape is a value: it reaches the parameter family
            // without the option family.
            ("argument.option.numeric", &["argument", "source"]),
            // A command kind falls back to the command family.
            ("command.builtin", &["command", "source"]),
            // A pager state falls back to the part, and the selected background
            // to the search-match attribute.
            (
                "selected.background",
                &["background", "@SearchMatch", "source"],
            ),
        ];

        for (path, expected) in cases {
            assert_eq!(chain_of(path), *expected, "the chain of `{path}`");
        }
    }

    #[test]
    fn a_products_absent_tuple_is_its_family_root() {
        assert_eq!(
            TokenScopeId::VARIABLE_DEFINITION.entry().path,
            "variable.definition"
        );
        assert_eq!(
            TokenScopeId::VARIABLE_DEFINITION_LOCAL.entry().path,
            "variable.definition.local"
        );
    }

    #[test]
    fn the_namespace_names_every_variable() {
        use fish_widestring::L;

        // A written stem gives the variable the configuration fixed.
        assert_eq!(
            TokenScopeId::NORMAL.entry().variable_name.as_ref(),
            L!("fish_color_normal")
        );
        assert_eq!(
            TokenScopeId::BUILTIN.entry().variable_name.as_ref(),
            L!("fish_color_builtin")
        );

        // A sum extends its family's stem, and its prefix when it writes one.
        assert_eq!(
            TokenScopeId::QUOTE_SINGLE.entry().variable_name.as_ref(),
            L!("fish_color_quote_single")
        );
        assert_eq!(
            TokenScopeId::KEYWORD_AND.entry().variable_name.as_ref(),
            L!("fish_color_keyword_and")
        );

        // A product's leaf extends its family's stem with its components.
        assert_eq!(
            TokenScopeId::VARIABLE_DEFINITION_LOCAL
                .entry()
                .variable_name
                .as_ref(),
            L!("fish_color_variable_definition_local")
        );

        // The pager's namespace is its own, over the same derivation.
        assert_eq!(
            TokenScopeId::SELECTED_BACKGROUND
                .entry()
                .variable_name
                .as_ref(),
            L!("fish_pager_color_selected_background")
        );
    }
}
