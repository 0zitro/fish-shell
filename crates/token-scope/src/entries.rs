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
    /// What the scope is, in the words the declaration states it in.
    ///
    /// The words are Markdown: the same description is the constant's own
    /// documentation and the entry's line in the generated documentation.
    pub description: &'static str,
    /// The variable that colors it.  Every entry has one: an entry is declared
    /// inside a namespace, and the namespace supplies the prefix.
    pub variable_name: ColorVariableName,
    /// The steps consulted for a character classified as this scope, most
    /// specific first, excluding this scope itself and ending at the root.
    pub resolution_chain: ScopeResolutionChain,
}

declare_scope_entries! {
    namespace "fish_color_" {
        "source" /** the default color */ normal root;
        "comment" /** comments like ``# important`` */;

        "keyword" /** keywords, falling back on the command color */ ["command"];
        sum "keyword" {
            begin /** the ``begin`` keyword */
            builtin /** the ``builtin`` decorator */
            case /** the ``case`` keyword */
            command /** the ``command`` decorator */
            else /** the ``else`` keyword */
            end /** the ``end`` keyword */
            exec /** the ``exec`` decorator */
            for /** the ``for`` keyword */
            function /** the ``function`` keyword */
            if /** the ``if`` keyword */
            in /** the ``in`` keyword */
            switch /** the ``switch`` keyword */
            while /** the ``while`` keyword */
        };
        "keyword.modifier" /** ``and``, ``or``, ``not``, ``!`` and ``time``, as a group */ ["operator"];
        sum "keyword.modifier" keyword {
            and /** the ``and`` keyword */
            or /** the ``or`` keyword */
            not /** the ``not`` keyword */
            exclam /** the ``!`` keyword */
            time /** the ``time`` decorator */
        };

        "command" /** commands, every kind falling back on this */;
        "command.external" /** a command found on the ``PATH`` */;
        "command.builtin" /** builtin commands like ``cd`` and ``set`` */ builtin;
        "command.function" /** user-defined functions */ function;
        "command.abbreviation" /** a command an abbreviation expands to */;
        "command.implicit_cd" /** a directory named in command position, which changes directory */;

        "argument" /** ordinary command parameters */ param;
        "argument.option" /** options starting with ``-``, up to the first ``--`` */ option;
        sum "argument.option" {
            short /** ``-`` and one character: ``-s`` */
            long /** ``--`` and a name: ``--long`` */
            gnu /** ``-`` and several characters: ``-option`` or ``-abc`` */
        };
        "argument.option.numeric" /** ``-`` and digits, which is a negative number rather than an option: ``-5`` */ option_numeric ["argument"];
        product "variable.definition" ["argument"] /** a name a command defines */ {
            sum { local /** local */ function /** function */ global /** global */ universal /** universal */ }
            sum { exported /** exported */ unexported /** unexported */ }
            sum { path /** a path variable */ non_path /** not a path variable */ }
        };

        "string.quoted" /** quoted text like ``"abc"`` */ quote;
        sum "string.quoted" {
            single /** the text of a single-quoted run */
            double /** the text of a double-quoted run */
        };
        sum "string.quoted.single" {
            delimiter /** the ``'`` marks around a single-quoted run */
        };
        sum "string.quoted.double" {
            delimiter /** the quote marks around a double-quoted run */
        };
        "string.escape" /** character escapes like ``\n`` and ``\x70`` */ escape;
        sum "string.escape" {
            unicode /** a ``\u``, ``\U``, ``\x`` or ``\X`` escape */
            octal /** a ``\`` and the octal digits after it */
            control /** a ``\c`` escape */
        };
        "string.escape.invalid" /** an escape whose value is out of range */ escape_invalid ["invalid"];

        "variable" /** a variable name in a reference */ ["operator"];
        "variable.sigil" /** the ``$`` before a name */;
        "variable.sigil.dereference" /** a ``$`` that dereferences the one after it */;
        "variable.slice" /** the brackets of a slice */;
        "variable.parameter" /** a parameter of the enclosing function definition */;
        "variable.parameter.argv" /** ``argv`` */;
        "variable.parameter.named" /** a name the enclosing definition declares with ``-a`` */;

        "operator" /** parameter expansion operators */;
        "operator.glob" /** ``*``, and ``?`` where a question mark globs */;
        "operator.brace" /** the ``{``, ``}`` and ``,`` of a brace expansion */;
        "operator.parenthesis" /** ``(`` and ``)`` */;
        "operator.tilde" /** ``~`` at the start of a word */;
        "operator.dollar" /** the ``$`` of ``$(command)`` */;
        "operator.logical" /** ``&&`` and ``||`` */;
        "operator.assignment" /** the ``=`` that separates a name from its value */;

        "redirection" /** IO redirections */;
        sum "redirection" {
            input /** a redirection that reads: ``<`` */
            output /** a redirection that writes: ``>`` */
            append /** a redirection that appends: ``>>`` */
            fd /** a redirection whose target is a file descriptor: ``>&`` */
        };

        "terminator" /** process separators like ``;`` and ``&`` */ end;
        sum "terminator" {
            pipe /** ``|`` */
            background /** ``&`` */
            semicolon /** ``;`` */
            newline /** a newline */
        };

        "invalid" /** syntax errors */ error;
        sum "invalid" {
            command /** a command word that names no command */
            path /** a ``cd`` argument that is not a path */
            redirection /** a redirection target that cannot be used */
        };

        // The scopes an enclosing construct contributes.  They are entries like the
        // rest; a character's set places them after its leaf's chain and before the
        // root, and what makes one enclosing is that position, not its declaration.
        "meta.function.no_shadowing" /** the body of a function definition whose header declares ``-S`` */ enclosing;
        "meta.function" /** the body of a function definition */ enclosing;
        "meta.command_substitution" /** the inside of a command substitution */ enclosing;
        "meta.block" /** the body of a ``begin`` block or a brace block */ enclosing;
        "meta.if" /** the body of an ``if``, ``else if`` or ``else`` branch */ enclosing;
        "meta.case" /** the body of a ``case`` clause */ enclosing;
        "meta.loop.for" /** a ``for`` loop's body */ enclosing;
        "meta.loop.while" /** a ``while`` loop's body */ enclosing;
        "meta.loop" /** every loop's body */ enclosing;
        "meta.pipeline" /** the commands of a pipeline */ enclosing;
        "meta.and_or" /** the commands of an ``and``/``or`` list */ enclosing;
    }

    namespace "fish_pager_color_" {
        "progress" /** the pager's status line, like ``and 3 more rows`` */;
        "background" /** the background of a row */;
        "prefix" /** the prefix of a row */;
        "completion" /** the completion text of a row */;
        "description" /** the description of a row */;
        "secondary.background" /** the background of a secondary row */ ["background"];
        "secondary.prefix" /** the prefix of a secondary row */ ["prefix"];
        "secondary.completion" /** the completion text of a secondary row */ ["completion"];
        "secondary.description" /** the description of a secondary row */ ["description"];
        "selected.background" /** the background of the selected row */ ["background", @SearchMatch];
        "selected.prefix" /** the prefix of the selected row */ ["prefix"];
        "selected.completion" /** the completion text of the selected row */ ["completion"];
        "selected.description" /** the description of the selected row */ ["description"];
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
    fn every_scope_states_what_it_is() {
        for entry in TOKEN_SCOPE_ENTRIES {
            assert!(
                !entry.description.is_empty(),
                "the scope `{}` states no description",
                entry.path
            );
            assert!(
                !entry.description.contains('\n'),
                "the description of `{}` is one line",
                entry.path
            );
        }
    }

    #[test]
    fn a_product_leaf_states_the_values_it_takes() {
        let leaf = TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED.entry();
        assert!(leaf.description.contains("local"), "{}", leaf.description);
        assert!(
            leaf.description.contains("exported"),
            "{}",
            leaf.description
        );

        let root = TokenScopeId::VARIABLE_DEFINITION.entry();
        assert!(
            !root.description.contains("local"),
            "the fully absent leaf states the family alone: {}",
            root.description
        );
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
