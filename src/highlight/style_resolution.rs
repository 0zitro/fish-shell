//! Resolution of a character's classification into the style the terminal is
//! given.
//!
//! A character carries an ordered set of scopes per color channel: the leaf scope the
//! visitor chose, the scopes its chain adds, the enclosing scopes it is inside, and
//! the root.
//!
//! Resolution composes their variables field by field.  Each variable is a partial
//! style: it may name a foreground, a background, an underline color, any of the
//! flags, or none of them.  A field named by a more specific scope replaces the same
//! field named by a less specific one; a field nobody names takes the terminal
//! default.  A scope that is not in the set is not consulted, so its fields are never
//! named and its rules apply by omission.

use std::collections::HashMap;
use std::collections::hash_map::Entry;

use fish_token_scope::{
    AttributeMode, ColorVariableName, EnclosingScopeSet, HighlightAttribute,
    HighlightAttributeEntry, ResolutionChainStep, StyleField, StyleFieldSet, TokenScopeId,
};

use crate::env::Environment;
use crate::highlight::HighlightClassification;
use crate::text_style::{
    ResettableStyle, SpecifiedTextStyle, TextStyle, UnderlineStyle, parse_text_style,
};

/// Which color channel of a classification one style is composed for.
///
/// The foreground channel composes the foreground color, the underline color and the
/// flags, and the background channel composes the background color.  A channel that
/// composed another channel's field would let one slot of a pager row write the
/// other slot's color.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ColorChannel {
    Foreground,
    Background,
}

impl ColorChannel {
    /// The fields of `value` this channel composes.
    fn fields_of(self, value: TextStyle) -> TextStyle {
        let fields = self.fields();
        value.restrict(fields)
    }

    /// The style with the fields this channel composes cleared.
    fn cleared(self, style: TextStyle) -> TextStyle {
        style.without(self.fields())
    }

    /// The fields this channel composes.
    fn fields(self) -> StyleFieldSet {
        match self {
            Self::Foreground => StyleFieldSet::of(&[
                StyleField::Foreground,
                StyleField::UnderlineColor,
                StyleField::Flags,
            ]),
            Self::Background => StyleFieldSet::of(&[StyleField::Background]),
        }
    }
}

/// Resolves highlight classifications to text styles.
///
/// The cache has no invalidation and its lifetime is one render: a resolver is
/// constructed by the screen for each update, as the previous resolver was.
#[derive(Default)]
pub struct HighlightStyleResolver {
    cache: HashMap<HighlightClassification, TextStyle>,
}

impl HighlightStyleResolver {
    /// Construct an empty resolver.
    pub fn new() -> Self {
        Self::default()
    }

    /// The style `classification` resolves to.
    ///
    /// Both channels are composed field by field, then the attributes combine with
    /// the result in `application_order`.
    pub fn style_of(
        &mut self,
        classification: HighlightClassification,
        vars: &dyn Environment,
    ) -> TextStyle {
        match self.cache.entry(classification) {
            Entry::Occupied(entry) => *entry.get(),
            Entry::Vacant(entry) => {
                let style = Self::resolve(classification, vars);
                *entry.insert(style)
            }
        }
    }

    fn resolve(classification: HighlightClassification, vars: &dyn Environment) -> TextStyle {
        let mut style = TextStyle::unknown();

        style = style.union_prefer_right(channel_style(
            ColorChannel::Foreground,
            classification.foreground(),
            classification.enclosing(),
            vars,
        ));

        if classification.background() != classification.foreground() {
            style = style.union_prefer_right(channel_style(
                ColorChannel::Background,
                classification.background(),
                classification.enclosing(),
                vars,
            ));
        }

        for entry in HighlightAttribute::entries() {
            if classification.attributes().has_attribute(entry.attribute) {
                style = combined_style(style, entry, vars);
            }
        }

        if classification
            .attributes()
            .has_attribute(HighlightAttribute::ForcedUnderline)
        {
            style
                .flags
                .inject_underline(ResettableStyle::On(UnderlineStyle::Single));
        }

        // Whatever no variable named is drawn as the terminal draws it.
        style.with_terminal_defaults()
    }
}

/// The style one color channel's scope set denotes.
///
/// The set is walked from the root toward the leaf — the root, the enclosing scopes
/// outermost first, the chain from its least specific step, and the leaf — and each
/// variable writes the fields the channel composes.  A variable carrying `--reset`
/// clears what was written for it before.
fn channel_style(
    channel: ColorChannel,
    scope: TokenScopeId,
    enclosing: EnclosingScopeSet,
    vars: &dyn Environment,
) -> TextStyle {
    let root = TokenScopeId::NORMAL;
    let chain: Vec<ResolutionChainStep> = scope.entry().resolution_chain.into_iter().collect();

    let mut steps: Vec<ResolutionChainStep> = Vec::with_capacity(chain.len() + 3);
    if scope != root {
        steps.push(ResolutionChainStep::Scope(root));
    }
    steps.extend(enclosing.scopes().rev().map(ResolutionChainStep::Scope));
    steps.extend(
        chain
            .into_iter()
            .rev()
            .filter(|step| *step != ResolutionChainStep::Scope(root)),
    );
    steps.push(ResolutionChainStep::Scope(scope));

    let mut style = TextStyle::unknown();
    for step in steps {
        let Some(variable) = step_variable(step) else {
            continue;
        };
        let Some(value) = variable_style(variable, vars) else {
            continue;
        };
        if value.reset {
            style = channel.cleared(style);
        }
        style = style.union_prefer_right(channel.fields_of(value.named_fields()));
    }

    style
}

/// The variable one chain step consults.
fn step_variable(step: ResolutionChainStep) -> Option<ColorVariableName> {
    match step {
        ResolutionChainStep::Scope(scope) => Some(scope.entry().variable_name),
        ResolutionChainStep::Attribute(attribute) => {
            attribute.entry().map(|entry| entry.variable_name)
        }
    }
}

/// The style one variable denotes, or none when it is unset, empty, or names
/// nothing.
fn variable_style(
    variable: ColorVariableName,
    vars: &dyn Environment,
) -> Option<SpecifiedTextStyle> {
    let value = vars.get_unless_empty(variable.as_ref())?;
    let specified = parse_text_style(value.as_list());
    (specified != SpecifiedTextStyle::default()).then_some(specified)
}

/// The style combined with an attribute's value under its mode.
fn combined_style(
    style: TextStyle,
    entry: &HighlightAttributeEntry,
    vars: &dyn Environment,
) -> TextStyle {
    match entry.mode {
        AttributeMode::Merge(fields) => {
            let Some(value) = variable_style(entry.variable_name, vars) else {
                return style;
            };
            let mut style = if value.reset {
                style.without(fields)
            } else {
                style
            };
            style = style.union_prefer_right(value.named_fields().restrict(fields));
            style
        }
        AttributeMode::Replace(fields) => {
            let replacement = variable_style(entry.variable_name, vars)
                .and_then(|value| value.find_style())
                .unwrap_or_else(TextStyle::terminal_default)
                .restrict(fields);
            style.union_prefer_right(replacement)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::HighlightStyleResolver;
    use crate::env::{EnvMode, EnvSetMode, EnvStack};
    use crate::highlight::{HighlightClassification, highlight_shell};
    use crate::operation_context::{EXPANSION_LIMIT_BACKGROUND, OperationContext};
    use crate::prelude::*;
    use crate::tests::prelude::*;
    use crate::text_style::{ResettableStyle, TextStyle, UnderlineStyle};
    use fish_color::Color;
    use fish_token_scope::{HighlightAttribute, TokenScopeId};

    /// The colour variables of one test, and the styles they resolve to.
    struct Colors {
        parser: TestParser,
    }

    impl Colors {
        fn new() -> Self {
            test_init();
            Self {
                parser: TestParser::new(),
            }
        }

        /// Set one colour variable to these `set_color` words.
        fn set(&self, name: &wstr, words: &[&wstr]) {
            let values = words.iter().map(|word| (*word).to_owned()).collect();
            self.vars()
                .set(name, EnvSetMode::new(EnvMode::LOCAL, false), values);
        }

        fn vars(&self) -> &EnvStack {
            self.parser.parser.vars()
        }

        fn style_of(&self, classification: HighlightClassification) -> TextStyle {
            HighlightStyleResolver::new().style_of(classification, self.vars())
        }

        /// The classification of every character of `text`.
        fn classify(&self, text: &wstr) -> Vec<HighlightClassification> {
            let mut classifications = vec![];
            highlight_shell(
                text,
                &mut classifications,
                &mut OperationContext::background(self.vars(), EXPANSION_LIMIT_BACKGROUND),
                true, /* io_ok */
                Some(text.len()),
            );
            classifications
        }
    }

    /// The named colour a `set_color` word denotes.
    fn color(name: &wstr) -> Color {
        Color::from_wstr(name).expect("the word names a colour")
    }

    #[test]
    #[serial]
    fn a_more_specific_scope_wins_field_by_field() {
        let colors = Colors::new();
        // The root names the flags, the family the underline, and the leaf the
        // colour.
        colors.set(L!("fish_color_normal"), &[L!("--bold")]);
        colors.set(L!("fish_color_command"), &[L!("--underline")]);
        colors.set(L!("fish_color_builtin"), &[L!("red")]);

        let style = colors.style_of(HighlightClassification::from(TokenScopeId::BUILTIN));

        assert_eq!(style.fg, color(L!("red")), "the leaf names the colour");
        assert!(
            style.flags.is_bold(),
            "a field only the root names still stands"
        );
        assert_eq!(
            style.flags.underline_style(),
            ResettableStyle::On(UnderlineStyle::Single),
            "a field only the family names still stands"
        );
        assert_eq!(
            style.bg,
            TextStyle::terminal_default().bg,
            "a field nobody names takes the terminal default"
        );
    }

    #[test]
    #[serial]
    fn a_reset_clears_what_the_less_specific_scopes_named() {
        let colors = Colors::new();
        colors.set(L!("fish_color_normal"), &[L!("--bold")]);
        colors.set(L!("fish_color_builtin"), &[L!("--reset"), L!("green")]);

        let style = colors.style_of(HighlightClassification::from(TokenScopeId::BUILTIN));

        assert!(
            !style.flags.is_bold(),
            "the reset clears what the root named"
        );
        assert_eq!(style.fg, color(L!("green")));
    }

    #[test]
    #[serial]
    fn an_empty_variable_names_nothing() {
        let colors = Colors::new();
        colors.set(L!("fish_color_command"), &[L!("red")]);
        colors.set(L!("fish_color_builtin"), &[]);

        let style = colors.style_of(HighlightClassification::from(TokenScopeId::BUILTIN));

        assert_eq!(
            style.fg,
            color(L!("red")),
            "the empty leaf contributes nothing, so the family colours it"
        );
    }

    #[test]
    #[serial]
    fn the_selection_replaces_the_style() {
        let colors = Colors::new();
        colors.set(L!("fish_color_comment"), &[L!("green"), L!("--underline")]);
        colors.set(
            L!("fish_color_selection"),
            &[L!("white"), L!("--background=brblack"), L!("--bold")],
        );

        let style = colors.style_of(
            HighlightClassification::from(TokenScopeId::COMMENT)
                .with_attribute(HighlightAttribute::Selection),
        );

        assert_eq!(style.fg, color(L!("white")));
        assert_eq!(style.bg, color(L!("brblack")));
        assert!(style.flags.is_bold());
        assert_eq!(
            style.flags.underline_style(),
            ResettableStyle::Off,
            "the composed underline is replaced, not kept"
        );
    }

    #[test]
    #[serial]
    fn the_search_match_supplies_the_fields_it_names() {
        let colors = Colors::new();
        colors.set(L!("fish_color_comment"), &[L!("green")]);
        colors.set(L!("fish_color_search_match"), &[L!("--background=brblue")]);

        let background_only = colors.style_of(
            HighlightClassification::from(TokenScopeId::COMMENT)
                .with_attribute(HighlightAttribute::SearchMatch),
        );
        assert_eq!(
            background_only.fg,
            color(L!("green")),
            "a match that names no foreground keeps the character's own"
        );
        assert_eq!(background_only.bg, color(L!("brblue")));

        colors.set(
            L!("fish_color_search_match"),
            &[L!("yellow"), L!("--background=brblue")],
        );
        let named_foreground = colors.style_of(
            HighlightClassification::from(TokenScopeId::COMMENT)
                .with_attribute(HighlightAttribute::SearchMatch),
        );
        assert_eq!(named_foreground.fg, color(L!("yellow")));
    }

    #[test]
    #[serial]
    fn the_autosuggestion_replaces_the_foreground_it_carries() {
        let colors = Colors::new();
        colors.set(L!("fish_color_comment"), &[L!("green")]);
        colors.set(L!("fish_color_autosuggestion"), &[L!("brblack")]);

        let style = colors.style_of(
            HighlightClassification::from(TokenScopeId::COMMENT)
                .with_attribute(HighlightAttribute::Autosuggestion),
        );

        assert_eq!(style.fg, color(L!("brblack")));
    }

    #[test]
    #[serial]
    fn a_pager_row_falls_back_through_the_search_match() {
        let colors = Colors::new();
        // The selected row's background falls back to the search-match attribute.
        colors.set(L!("fish_color_search_match"), &[L!("--background=brblue")]);

        let row = HighlightClassification::of_scopes(
            TokenScopeId::COMPLETION,
            TokenScopeId::SELECTED_BACKGROUND,
        );
        assert_eq!(colors.style_of(row).bg, color(L!("brblue")));

        // Its own variable wins over the fallback.
        colors.set(
            L!("fish_pager_color_selected_background"),
            &[L!("--background=red")],
        );
        assert_eq!(colors.style_of(row).bg, color(L!("red")));
    }

    #[test]
    #[serial]
    fn the_forced_underline_is_a_fixed_effect() {
        let colors = Colors::new();

        let style = colors.style_of(
            HighlightClassification::default().with_attribute(HighlightAttribute::ForcedUnderline),
        );

        assert_eq!(
            style.flags.underline_style(),
            ResettableStyle::On(UnderlineStyle::Single)
        );
    }

    #[test]
    #[serial]
    fn a_quote_style_colours_its_run_and_its_delimiters() {
        let colors = Colors::new();
        colors.set(L!("fish_color_quote"), &[L!("yellow")]);
        colors.set(L!("fish_color_quote_single"), &[L!("green")]);

        // echo 'ab' "cd"
        let classifications = colors.classify(L!("echo 'ab' \"cd\""));
        let style = |index: usize| colors.style_of(classifications[index]);

        assert_eq!(
            style(5).fg,
            color(L!("green")),
            "the single run's opening mark"
        );
        assert_eq!(style(6).fg, color(L!("green")), "the single run's content");
        assert_eq!(
            style(8).fg,
            color(L!("green")),
            "the single run's closing mark"
        );
        assert_eq!(
            style(10).fg,
            color(L!("yellow")),
            "the double run keeps the family's colour"
        );

        // Setting the delimiter stem alone splits the marks off the content.
        colors.set(L!("fish_color_quote_single_delimiter"), &[L!("brred")]);
        let classifications = colors.classify(L!("echo 'ab' \"cd\""));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(5).fg, color(L!("brred")), "the opening mark");
        assert_eq!(style(6).fg, color(L!("green")), "the content is untouched");
    }

    #[test]
    #[serial]
    fn a_quoted_run_does_not_inherit_the_arguments_colour() {
        let colors = Colors::new();
        colors.set(L!("fish_color_param"), &[L!("red")]);

        // echo x 'ab'
        let classifications = colors.classify(L!("echo x 'ab'"));
        let terminal_default = TextStyle::terminal_default();

        assert_eq!(
            colors.style_of(classifications[5]).fg,
            color(L!("red")),
            "the argument is coloured"
        );
        assert_eq!(
            colors.style_of(classifications[8]).fg,
            terminal_default.fg,
            "the quoted content reaches the root without the argument family"
        );
    }

    #[test]
    #[serial]
    fn an_escape_resolves_at_the_escape_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_quote_double"), &[L!("green")]);
        colors.set(L!("fish_color_escape"), &[L!("brblue")]);

        // echo "a\\b" — a backslash is escape only before the characters the quotes
        // keep special, and `\\` is one of them.
        let classifications = colors.classify(L!("echo \"a\\\\b\""));
        let style = |index: usize| colors.style_of(classifications[index]);

        assert_eq!(style(6).fg, color(L!("green")), "the quoted content");
        assert_eq!(style(7).fg, color(L!("brblue")), "the backslash");
        assert_eq!(style(8).fg, color(L!("brblue")), "the escaped character");

        // An escape class is a refinement of the family, so it reaches it.
        let classifications = colors.classify(L!("echo \\u1234"));
        assert_eq!(colors.style_of(classifications[5]).fg, color(L!("brblue")));
    }

    #[test]
    #[serial]
    fn an_operator_shape_resolves_through_the_operator_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_operator"), &[L!("brblue")]);

        // echo * ~/src {a,b} (cmd) — the shapes agree with the family until one is set.
        let classifications = colors.classify(L!("echo * ~/src {a,b} (cmd)"));
        for index in [5, 7, 13, 15, 17, 19, 23] {
            assert_eq!(
                colors.style_of(classifications[index]).fg,
                color(L!("brblue")),
                "the operator at {index}"
            );
        }

        // Its own variable splits one shape off.
        colors.set(L!("fish_color_operator_glob"), &[L!("green")]);
        let classifications = colors.classify(L!("echo *"));
        assert_eq!(colors.style_of(classifications[5]).fg, color(L!("green")));
    }

    #[test]
    #[serial]
    fn a_keyword_resolves_through_its_own_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_command"), &[L!("red")]);

        // if true; end — a keyword falls back on the command family, as the table says.
        let classifications = colors.classify(L!("if true; end"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(0).fg, color(L!("red")));
        assert_eq!(style(9).fg, color(L!("red")));

        // Its own family stands in front of that fallback.
        colors.set(L!("fish_color_keyword"), &[L!("green")]);
        let classifications = colors.classify(L!("if true; end"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(0).fg, color(L!("green")));
        assert_eq!(style(9).fg, color(L!("green")));

        // A refinement splits one keyword off.
        colors.set(L!("fish_color_keyword_if"), &[L!("yellow")]);
        let classifications = colors.classify(L!("if true; end"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(0).fg, color(L!("yellow")));
        assert_eq!(style(9).fg, color(L!("green")));

        // A modifier is under the modifier family, which reaches the operator family.
        colors.set(L!("fish_color_operator"), &[L!("brblue")]);
        let classifications = colors.classify(L!("true; and false"));
        assert_eq!(colors.style_of(classifications[6]).fg, color(L!("brblue")));
    }

    #[test]
    #[serial]
    fn the_sigil_and_its_dereferences_split_from_the_name() {
        let colors = Colors::new();
        colors.set(L!("fish_color_variable"), &[L!("green")]);

        // echo $$x — both dollars resolve through the variable family first.
        let classifications = colors.classify(L!("echo $$x"));
        for (index, classification) in classifications.iter().enumerate().take(8).skip(5) {
            assert_eq!(
                colors.style_of(*classification).fg,
                color(L!("green")),
                "the reference at {index}"
            );
        }

        // The sigil splits the dollars off the name.
        colors.set(L!("fish_color_variable_sigil"), &[L!("brblue")]);
        let classifications = colors.classify(L!("echo $$x"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(
            style(5).fg,
            color(L!("brblue")),
            "a dereferencing dollar falls back on the sigil"
        );
        assert_eq!(style(6).fg, color(L!("brblue")), "the sigil");
        assert_eq!(style(7).fg, color(L!("green")), "the name is untouched");

        // The dereference splits only the dollars that dereference.
        colors.set(L!("fish_color_variable_sigil_dereference"), &[L!("yellow")]);
        let classifications = colors.classify(L!("echo $$x"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(5).fg, color(L!("yellow")));
        assert_eq!(style(6).fg, color(L!("brblue")));
    }

    #[test]
    #[serial]
    fn a_parameter_resolves_against_the_definitions_header() {
        let colors = Colors::new();
        colors.set(L!("fish_color_variable"), &[L!("green")]);
        colors.set(L!("fish_color_variable_parameter"), &[L!("brblue")]);

        // function f -a x; echo $x $argv $y; end
        let classifications = colors.classify(L!("function f -a x; echo $x $argv $y; end"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(23).fg, color(L!("brblue")), "the declared name");
        assert_eq!(style(26).fg, color(L!("brblue")), "argv");
        assert_eq!(
            style(32).fg,
            color(L!("green")),
            "a name the header does not declare"
        );

        // Outside a definition's body the same name is no parameter.
        let classifications = colors.classify(L!("echo $x"));
        assert_eq!(colors.style_of(classifications[6]).fg, color(L!("green")));

        // A nested definition resolves against the innermost list.
        let classifications =
            colors.classify(L!("function f -a x; function g -a y; echo $x $y; end; end"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(
            style(40).fg,
            color(L!("green")),
            "the outer definition's name is not a parameter of the inner one"
        );
        assert_eq!(
            style(43).fg,
            color(L!("brblue")),
            "the inner definition's name"
        );
    }

    #[test]
    #[serial]
    fn an_option_shape_resolves_through_the_option_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_param"), &[L!("red")]);
        colors.set(L!("fish_color_option"), &[L!("green")]);

        // echo -s --long -gnu -5
        let classifications = colors.classify(L!("echo -s --long -gnu -5"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(5).fg, color(L!("green")), "the short shape");
        assert_eq!(style(8).fg, color(L!("green")), "the long shape");
        assert_eq!(style(15).fg, color(L!("green")), "the GNU shape");
        assert_eq!(
            style(20).fg,
            color(L!("red")),
            "a negative number is a value, so it reaches the parameter"
        );

        // Its own variable splits one shape off.
        colors.set(L!("fish_color_option_long"), &[L!("brblue")]);
        let classifications = colors.classify(L!("echo -s --long -gnu -5"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(5).fg, color(L!("green")));
        assert_eq!(style(8).fg, color(L!("brblue")));
    }

    #[test]
    #[serial]
    fn a_long_options_value_is_an_argument() {
        let colors = Colors::new();
        colors.set(L!("fish_color_option"), &[L!("green")]);
        colors.set(L!("fish_color_param"), &[L!("red")]);

        // echo --colour=always
        let classifications = colors.classify(L!("echo --colour=always"));
        let style = |index: usize| colors.style_of(classifications[index]);
        assert_eq!(style(5).fg, color(L!("green")), "the name");
        assert_eq!(
            style(14).fg,
            color(L!("red")),
            "the value after the `=` is an argument"
        );
    }

    #[test]
    #[serial]
    fn a_command_kind_resolves_through_the_command_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_command"), &[L!("green")]);

        // A builtin and an external program agree until a kind is set.
        let builtin = colors.classify(L!("echo"));
        let external = colors.classify(L!("/bin/cat"));
        assert_eq!(
            colors.style_of(builtin[0]).fg,
            color(L!("green")),
            "the builtin"
        );
        assert_eq!(
            colors.style_of(external[0]).fg,
            color(L!("green")),
            "the external program"
        );

        // The external kind splits off without touching the builtin.
        colors.set(L!("fish_color_command_external"), &[L!("brblue")]);
        let builtin = colors.classify(L!("echo"));
        let external = colors.classify(L!("/bin/cat"));
        assert_eq!(colors.style_of(builtin[0]).fg, color(L!("green")));
        assert_eq!(colors.style_of(external[0]).fg, color(L!("brblue")));

        // A word that no command-position test matches takes the invalid family.
        let classifications = colors.classify(L!("/bin/definitely_not_a_command"));
        assert_eq!(classifications[0].foreground(), TokenScopeId::ERROR_COMMAND);
    }

    #[test]
    #[serial]
    fn an_enclosing_scope_falls_back_after_the_leaf() {
        let brblue = color(L!("brblue"));

        // echo a | cat — the pipeline encloses both commands, and its variable supplies a
        // field the leaf's chain names nothing for.
        let colors = Colors::new();
        colors.set(L!("fish_color_meta_pipeline"), &[L!("brblue")]);
        let classifications = colors.classify(L!("echo a | cat"));
        assert_eq!(colors.style_of(classifications[0]).fg, brblue);
        assert_eq!(colors.style_of(classifications[9]).fg, brblue);

        // A command outside the pipeline is inside no such scope.
        let classifications = colors.classify(L!("echo a"));
        assert_ne!(colors.style_of(classifications[0]).fg, brblue);

        // The leaf's own chain precedes the enclosing scope, so it wins a field it names.
        colors.set(L!("fish_color_command"), &[L!("green")]);
        let classifications = colors.classify(L!("echo a | cat"));
        assert_eq!(colors.style_of(classifications[0]).fg, color(L!("green")));
    }

    #[test]
    #[serial]
    fn a_definition_resolves_through_its_requested_mode() {
        let colors = Colors::new();
        colors.set(L!("fish_color_variable_definition"), &[L!("green")]);

        // set x / set -l x — a name whose mode names no scope, and one whose mode does.
        let plain = colors.classify(L!("set x 1"));
        let local = colors.classify(L!("set -l x 1"));
        assert_eq!(colors.style_of(plain[4]).fg, color(L!("green")));
        assert_eq!(colors.style_of(local[7]).fg, color(L!("green")));

        // The scope leaf splits the requested scope off.
        colors.set(L!("fish_color_variable_definition_local"), &[L!("brblue")]);
        let plain = colors.classify(L!("set x 1"));
        let local = colors.classify(L!("set -l x 1"));
        assert_eq!(colors.style_of(plain[4]).fg, color(L!("green")));
        assert_eq!(colors.style_of(local[7]).fg, color(L!("brblue")));

        // And the export leaf extends it.
        colors.set(
            L!("fish_color_variable_definition_local_exported"),
            &[L!("yellow")],
        );
        let exported = colors.classify(L!("set -lx x 1"));
        assert_eq!(colors.style_of(exported[8]).fg, color(L!("yellow")));

        // A name a loop defines is local, and so is one a header declares.
        let loop_name = colors.classify(L!("for x in a; end"));
        assert_eq!(colors.style_of(loop_name[4]).fg, color(L!("brblue")));
    }

    #[test]
    #[serial]
    fn a_redirection_and_a_terminator_resolve_through_their_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_redirection"), &[L!("green")]);
        colors.set(L!("fish_color_end"), &[L!("green")]);

        let input = colors.classify(L!("true < f"));
        let append = colors.classify(L!("true >> f"));
        let pipe = colors.classify(L!("true | true"));
        let semicolon = colors.classify(L!("true; true"));
        let fg = |classifications: &[HighlightClassification], index: usize| {
            colors.style_of(classifications[index]).fg
        };
        assert_eq!(fg(&input, 5), color(L!("green")), "input");
        assert_eq!(fg(&append, 5), color(L!("green")), "append");
        assert_eq!(fg(&pipe, 5), color(L!("green")), "pipe");
        assert_eq!(fg(&semicolon, 4), color(L!("green")), "semicolon");

        // A kind names itself.
        colors.set(L!("fish_color_redirection_append"), &[L!("brblue")]);
        colors.set(L!("fish_color_end_pipe"), &[L!("brblue")]);
        let input = colors.classify(L!("true < f"));
        let append = colors.classify(L!("true >> f"));
        let pipe = colors.classify(L!("true | true"));
        let semicolon = colors.classify(L!("true; true"));
        let fg = |classifications: &[HighlightClassification], index: usize| {
            colors.style_of(classifications[index]).fg
        };
        assert_eq!(fg(&input, 5), color(L!("green")), "input stays");
        assert_eq!(fg(&append, 5), color(L!("brblue")), "append splits");
        assert_eq!(fg(&pipe, 5), color(L!("brblue")), "pipe splits");
        assert_eq!(fg(&semicolon, 4), color(L!("green")), "semicolon stays");
    }

    #[test]
    #[serial]
    fn a_builtin_word_and_a_keyword_word_resolve_through_their_family() {
        let colors = Colors::new();
        colors.set(L!("fish_color_builtin"), &[L!("green")]);
        colors.set(L!("fish_color_keyword"), &[L!("green")]);
        let fg = |text: &wstr| {
            let classifications = colors.classify(text);
            colors.style_of(classifications[0]).fg
        };

        assert_eq!(
            colors.classify(L!("true"))[0].foreground(),
            TokenScopeId::BUILTIN_BOOLEAN_TRUE
        );
        assert_eq!(fg(L!("true")), color(L!("green")), "a builtin word");
        assert_eq!(fg(L!("false")), color(L!("green")), "another builtin word");
        assert_eq!(fg(L!("return")), color(L!("green")), "a keyword word");

        // A category splits its words from the builtin family, and a word splits from the
        // category.
        colors.set(L!("fish_color_builtin_boolean"), &[L!("brred")]);
        assert_eq!(fg(L!("true")), color(L!("brred")), "the category");
        assert_eq!(fg(L!("false")), color(L!("brred")), "the category");
        colors.set(L!("fish_color_builtin_boolean_true"), &[L!("brblue")]);
        assert_eq!(fg(L!("true")), color(L!("brblue")), "the word");
        assert_eq!(fg(L!("false")), color(L!("brred")), "the category below it");

        // A keyword's own variable splits it from the keyword family.
        colors.set(L!("fish_color_keyword_return"), &[L!("brblue")]);
        assert_eq!(fg(L!("return")), color(L!("brblue")), "the keyword");
        assert_eq!(fg(L!("set")), color(L!("green")), "a builtin word");
    }

    #[test]
    #[serial]
    fn a_cached_style_is_the_style_a_fresh_resolver_gives() {
        let colors = Colors::new();
        colors.set(L!("fish_color_command"), &[L!("red"), L!("--bold")]);
        colors.set(L!("fish_color_operator"), &[L!("yellow")]);

        let classifications = [
            HighlightClassification::from(TokenScopeId::BUILTIN),
            HighlightClassification::from(TokenScopeId::OPERATOR),
            HighlightClassification::of_scopes(
                TokenScopeId::COMPLETION,
                TokenScopeId::SELECTED_BACKGROUND,
            ),
        ];

        let vars = colors.vars();
        let mut shared = HighlightStyleResolver::new();
        for classification in classifications {
            let cached = shared.style_of(classification, vars);
            let fresh = HighlightStyleResolver::new().style_of(classification, vars);
            assert_eq!(
                cached, fresh,
                "a style read from the cache is the style a fresh resolution gives"
            );
        }
    }

    #[test]
    #[serial]
    fn trailing_spaces_do_not_inherit_the_commands_underline() {
        let colors = Colors::new();
        colors.set(L!("fish_color_command"), &[L!("--underline")]);

        let text = L!("echo   ").to_owned();
        let mut classifications = vec![];
        highlight_shell(
            &text,
            &mut classifications,
            &mut OperationContext::background(colors.vars(), EXPANSION_LIMIT_BACKGROUND),
            true, /* io_ok */
            Some(text.len()),
        );
        assert_eq!(classifications.len(), text.len());

        let vars = colors.vars();
        let mut resolver = HighlightStyleResolver::new();
        for (index, classification) in classifications.iter().enumerate() {
            let underline = resolver
                .style_of(*classification, vars)
                .flags
                .underline_style();
            let expected = if index < 4 {
                ResettableStyle::On(UnderlineStyle::Single)
            } else {
                ResettableStyle::Off
            };
            assert_eq!(underline, expected, "character {index} of {text:?}");
        }
    }
}
