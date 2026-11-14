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
        "source" /** default color */ normal root;
        "comment" /** comments like ``# important`` */;

        "keyword" /** keywords like ``if``, ``for`` and ``end`` */ ["command"];
        sum "keyword" {
            begin /** ``begin`` keyword */
            break /** ``break`` keyword */
            builtin /** ``builtin`` decorator */
            case /** ``case`` keyword */
            command /** ``command`` decorator */
            continue /** ``continue`` keyword */
            else /** ``else`` keyword */
            end /** ``end`` keyword */
            exec /** ``exec`` decorator */
            for /** ``for`` keyword */
            function /** ``function`` keyword */
            if /** ``if`` keyword */
            in /** ``in`` keyword */
            return /** ``return`` keyword */
            switch /** ``switch`` keyword */
            while /** ``while`` keyword */
        };
        "keyword.modifier" /** ``and``, ``or``, ``not``, ``!`` and ``time`` */ ["operator"];
        sum "keyword.modifier" keyword {
            and /** ``and`` */
            or /** ``or`` */
            not /** ``not`` */
            exclam /** ``!`` */
            time /** ``time`` decorator */
        };

        "command" /** commands like ``echo`` */;
        "command.external" /** programs on the ``PATH``, like ``/bin/cat`` */;
        "command.builtin" /** builtins like ``cd`` and ``echo`` */ builtin;
        "command.builtin.async" /** the ``emit`` and ``wait`` builtins */ builtin_async;
        sum "command.builtin.async" builtin_async {
            emit /** ``emit`` builtin */
            wait /** ``wait`` builtin */
        };
        "command.builtin.boolean" /** the ``true`` and ``false`` builtins */ builtin_boolean;
        sum "command.builtin.boolean" builtin_boolean {
            false /** ``false`` builtin */
            true /** ``true`` builtin */
        };
        "command.builtin.eval" /** ``eval`` builtin */ builtin_eval;
        "command.builtin.random" /** ``random`` builtin */ builtin_random;
        "command.builtin.read" /** ``read`` builtin */ builtin_read;
        "command.builtin.set" /** ``set`` builtin */ builtin_set;
        "command.builtin.status" /** ``status`` builtin */ builtin_status;
        "command.builtin.test" /** ``test`` builtin */ builtin_test;
        "command.function" /** function calls like ``myfunc`` */ function;
        "command.abbreviation" /** abbreviations, like ``gco`` for ``git checkout`` */;
        "command.implicit_cd" /** directories as command words, like ``/tmp`` */;

        "argument" /** ordinary command parameters */ param;
        "argument.option" /** options like ``-l`` and ``--long`` */ option;
        sum "argument.option" {
            short /** one-letter options like ``-l`` */
            long /** named options like ``--long`` */
            gnu /** several letters after one dash, like ``-abc`` */
        };
        "argument.option.numeric" /** negative numbers like ``-5`` */ option_numeric ["argument"];
        product "variable.definition" ["argument"] /** ``x`` in ``set{}{}{} x`` */ {
            sum { local " -l" function " -f" global " -g" universal " -U" }
            sum { exported " -x" unexported " -u" }
            sum { path " --path" non_path " --unpath" }
        };

        "string.quoted" /** quoted text like ``"abc"`` */ quote;
        sum "string.quoted" {
            single /** text inside ``'...'`` */
            double /** text inside ``"..."`` */
        };
        sum "string.quoted.single" {
            delimiter /** ``'`` quotes of a single-quoted run */
        };
        sum "string.quoted.double" {
            delimiter /** ``"`` quotes of a double-quoted run */
        };
        "string.escape" /** character escapes like ``\n`` and ``\x70`` */ escape;
        sum "string.escape" {
            unicode /** ``\u``, ``\U``, ``\x`` and ``\X`` escapes */
            octal /** ``\`` and the octal digits after it */
            control /** ``\c`` escapes */
        };
        "string.escape.invalid" /** escapes out of range, like ``\U110000`` */ escape_invalid ["invalid"];

        "variable" /** variable names like ``x`` in ``$x`` */ ["operator"];
        "variable.sigil" /** ``$`` before a variable name */;
        "variable.sigil.dereference" /** ``$`` dereferencing the next one, as in ``$$x`` */;
        "variable.slice" /** slice brackets like ``[1]`` in ``$x[1]`` */;
        "variable.parameter" /** ``$x`` referring to a parameter inside ``function … end`` */;
        "variable.parameter.argv" /** ``$argv`` */;
        "variable.parameter.named" /** ``$x`` referring to a name from ``-a`` */;

        "operator" /** expansion operators like ``*`` and ``~`` */;
        "operator.glob" /** ``*``, and ``?`` where a question mark globs */;
        "operator.brace" /** ``{``, ``}`` and the ``,`` between them */;
        "operator.parenthesis" /** ``(`` and ``)`` */;
        "operator.tilde" /** ``~`` at the start of a word */;
        "operator.dollar" /** ``$`` of ``$(command)`` */;
        "operator.logical" /** ``&&`` and ``||`` */;
        "operator.assignment" /** ``=`` between a name and its value */;

        "redirection" /** redirections like ``>/dev/null`` */;
        sum "redirection" {
            input /** ``<`` and its target */
            output /** ``>`` and its target */
            append /** ``>>`` and its target */
            fd /** ``>&`` and the descriptor after it */
        };

        "terminator" /** separators like ``;``, ``&`` and ``|`` */ end;
        sum "terminator" {
            pipe /** ``|`` */
            background /** ``&`` */
            semicolon /** ``;`` */
            newline /** newline */
        };

        "invalid" /** syntax errors */ error;
        sum "invalid" {
            command /** unknown command words, like ``/bin/nope`` */
            path /** non-directory ``cd`` arguments, like ``cd foo`` */
            redirection /** unusable redirection targets, like ``> /nope/x`` */
        };

        // The scopes an enclosing construct contributes.  They are entries like the
        // rest; a character's set places them after its leaf's chain and before the
        // root, and what makes one enclosing is that position, not its declaration.
        "meta.function.no_shadowing" /** characters inside ``function … end -S`` */ enclosing;
        "meta.function" /** characters inside ``function … end`` */ enclosing;
        "meta.command_substitution" /** characters inside ``$(…)`` */ enclosing;
        "meta.block" /** characters inside ``begin … end`` */ enclosing;
        "meta.if" /** characters inside ``if … end`` */ enclosing;
        "meta.case" /** characters inside ``case … end`` */ enclosing;
        "meta.loop.for" /** characters inside ``for … end`` */ enclosing;
        "meta.loop.while" /** characters inside ``while … end`` */ enclosing;
        "meta.loop" /** characters inside ``for … end`` and ``while … end`` */ enclosing;
        "meta.pipeline" /** characters of ``echo | cat`` */ enclosing;
        "meta.and_or" /** characters of ``true && echo`` */ enclosing;
    }

    namespace "fish_pager_color_" {
        "progress" /** status line of the pager, like ``and 3 more rows`` */;
        "background" /** background color of a line */;
        "prefix" /** prefix string to be completed */;
        "completion" /** completion text, i.e. the proposed rest of the string */;
        "description" /** completion description */;
        "secondary.background" /** background of every second unselected completion */ ["background"];
        "secondary.prefix" /** prefix of every second unselected completion */ ["prefix"];
        "secondary.completion" /** suffix of every second unselected completion */ ["completion"];
        "secondary.description" /** description of every second unselected completion */ ["description"];
        "selected.background" /** background of the selected completion */ ["background", @SearchMatch];
        "selected.prefix" /** prefix of the selected completion */ ["prefix"];
        "selected.completion" /** suffix of the selected completion */ ["completion"];
        "selected.description" /** description of the selected completion */ ["description"];
    }
}

/// The scope a builtin's word names, when the declaration gives the word one.
///
/// The declaration states the word as the last component of a builtin's path —
/// `command.builtin.set` declares ``set``, and `command.builtin.async.emit`
/// declares ``emit`` — so the words with a scope of their own are read from the
/// paths rather than listed beside them.  A path with children states a category
/// of builtins, not a word: no builtin is named after one.
pub fn builtin_word_scope(word: &wstr) -> Option<TokenScopeId> {
    /// The path prefix whose remainder ends in a builtin's word.
    const PREFIX: &str = "command.builtin.";

    scope_of_word_under(word, PREFIX)
}

/// The scope a keyword's word names, when the declaration gives the word one.
///
/// The declaration states the word as the last component of a keyword's path —
/// `keyword.return` declares ``return`` — so the words with a scope of their own
/// are read from the paths rather than listed beside them.  A modifier's word is
/// not reached this way: it is carried by the decoration that spells it, not by a
/// command word.
pub fn keyword_word_scope(word: &wstr) -> Option<TokenScopeId> {
    /// The path prefix whose remainder is a keyword's word.
    const PREFIX: &str = "keyword.";

    scope_of_word_under(word, PREFIX)
}

/// The scope whose path under `prefix` ends in `word`.
fn scope_of_word_under(word: &wstr, prefix: &str) -> Option<TokenScopeId> {
    TOKEN_SCOPE_ENTRIES.iter().find_map(|entry| {
        let path = entry.path.strip_prefix(prefix)?;
        let last = path.rsplit('.').next().expect("every path has a component");
        last.chars().eq(word.chars()).then_some(entry.scope)
    })
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
    fn a_product_leaf_states_the_spelling_it_answers_to() {
        let description = |scope: TokenScopeId| scope.entry().description;

        assert_eq!(
            description(TokenScopeId::VARIABLE_DEFINITION),
            "``x`` in ``set x``"
        );
        assert_eq!(
            description(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED),
            "``x`` in ``set -l -x x``"
        );
        assert_eq!(
            description(TokenScopeId::VARIABLE_DEFINITION_UNIVERSAL_UNEXPORTED_NON_PATH),
            "``x`` in ``set -U -u --unpath x``"
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
