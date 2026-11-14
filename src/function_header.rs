//! The options a function definition's header accepts.
//!
//! The definition builtin parses its argument list against this table, and the
//! classifier reads a definition's header out of the buffer against the same one; a flag
//! renamed here cannot keep being read under an old spelling.  Each option states
//! whether it takes an argument, because the word after a flag that takes one is that
//! flag's value and not another flag: in `function f --description -S` the `-S` is the
//! description, and the definition shadows its scope as usual.

use fish_wgetopt::{ArgType, NON_OPTION_CHAR, WGetopter, WOption, wopt};
use fish_widestring::{L, WString, wstr};

/// What an option of a function header declares about the function.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FunctionHeaderEffect {
    /// The function's description.
    Description,
    /// One of the function's argument names.
    ArgumentNames,
    /// The function's body shares the calling scope.
    NoScopeShadowing,
    /// The variable a function inherits.
    InheritVariable,
    /// The command the function wraps.
    Wraps,
    /// An event the function handles.
    Event,
    /// The option prints the help text.
    Help,
}

/// One option of a function header and what it declares.
#[derive(Clone, Copy, Debug)]
pub struct FunctionHeaderFlag {
    /// The short spelling, as `wgetopt` takes it.
    pub short: char,
    /// The long spelling.
    pub long: &'static wstr,
    /// Whether the option takes a required argument.
    pub argument: bool,
    /// What the option declares about the function.
    pub effect: FunctionHeaderEffect,
}

impl FunctionHeaderFlag {
    /// This option as a `wgetopt` option.
    pub fn option(self) -> WOption<'static> {
        wopt(
            self.long,
            if self.argument {
                ArgType::RequiredArgument
            } else {
                ArgType::NoArgument
            },
            self.short,
        )
    }
}

/// Every option a function header accepts, in one table.
pub static FUNCTION_HEADER_FLAGS: &[FunctionHeaderFlag] = &[
    FunctionHeaderFlag {
        short: 'a',
        long: L!("argument-names"),
        argument: true,
        effect: FunctionHeaderEffect::ArgumentNames,
    },
    FunctionHeaderFlag {
        short: 'd',
        long: L!("description"),
        argument: true,
        effect: FunctionHeaderEffect::Description,
    },
    FunctionHeaderFlag {
        short: 'e',
        long: L!("on-event"),
        argument: true,
        effect: FunctionHeaderEffect::Event,
    },
    FunctionHeaderFlag {
        short: 'h',
        long: L!("help"),
        argument: false,
        effect: FunctionHeaderEffect::Help,
    },
    FunctionHeaderFlag {
        short: 'j',
        long: L!("on-job-exit"),
        argument: true,
        effect: FunctionHeaderEffect::Event,
    },
    FunctionHeaderFlag {
        short: 'p',
        long: L!("on-process-exit"),
        argument: true,
        effect: FunctionHeaderEffect::Event,
    },
    FunctionHeaderFlag {
        short: 's',
        long: L!("on-signal"),
        argument: true,
        effect: FunctionHeaderEffect::Event,
    },
    FunctionHeaderFlag {
        short: 'S',
        long: L!("no-scope-shadowing"),
        argument: false,
        effect: FunctionHeaderEffect::NoScopeShadowing,
    },
    FunctionHeaderFlag {
        short: 'v',
        long: L!("on-variable"),
        argument: true,
        effect: FunctionHeaderEffect::Event,
    },
    FunctionHeaderFlag {
        short: 'V',
        long: L!("inherit-variable"),
        argument: true,
        effect: FunctionHeaderEffect::InheritVariable,
    },
    FunctionHeaderFlag {
        short: 'w',
        long: L!("wraps"),
        argument: true,
        effect: FunctionHeaderEffect::Wraps,
    },
];

/// The `wgetopt` short-option string the header's options spell.
///
/// The leading `-` selects the order the definition builtin reads its argument list in:
/// options and words come back as they were written, so a word a `--argument-names`
/// starts stays a name.  Each option that takes an argument carries `:`.
pub fn header_short_options() -> WString {
    let mut short = WString::from(L!("-"));
    for flag in FUNCTION_HEADER_FLAGS {
        short.push(flag.short);
        if flag.argument {
            short.push(':');
        }
    }
    short
}

/// The header's options as `wgetopt` long options.
pub fn header_options() -> Vec<WOption<'static>> {
    FUNCTION_HEADER_FLAGS
        .iter()
        .map(|flag| flag.option())
        .collect()
}

/// The facts a definition's header words declare.
///
/// The first word is the function's name, which `wgetopt` takes to be the program name;
/// the rest are its options and the names and values they take.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct FunctionHeaderFacts {
    /// The argument names the header declares, in the order it declares them.
    pub argument_names: Vec<WString>,
    /// Whether the body shares the calling scope.
    pub no_scope_shadowing: bool,
}

impl FunctionHeaderFacts {
    /// The facts the header's words declare.
    pub fn of_header_words(words: &[&wstr]) -> Self {
        let mut argv: Vec<&wstr> = words.to_vec();
        let short_options = header_short_options();
        let options = header_options();
        let mut getopter = WGetopter::new(&short_options, &options, &mut argv);

        let mut facts = Self::default();
        let mut declaring_names = false;
        while let Some(value) = getopter.next_opt() {
            let flag = FUNCTION_HEADER_FLAGS
                .iter()
                .find(|flag| flag.short == value)
                .copied();
            match flag.map(|flag| flag.effect) {
                Some(FunctionHeaderEffect::ArgumentNames) => {
                    declaring_names = true;
                    facts.push_name(getopter.woptarg);
                }
                Some(FunctionHeaderEffect::NoScopeShadowing) => {
                    declaring_names = false;
                    facts.no_scope_shadowing = true;
                }
                Some(_) => declaring_names = false,
                // A word that is no option is a name once an argument-names option has
                // introduced the names, and nothing otherwise.
                None if value == NON_OPTION_CHAR => {
                    if declaring_names {
                        facts.push_name(getopter.woptarg);
                    }
                }
                None => declaring_names = false,
            }
        }
        facts
    }

    fn push_name(&mut self, name: Option<&wstr>) {
        if let Some(name) = name {
            self.argument_names.push(name.to_owned());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::FunctionHeaderFacts;
    use fish_widestring::L;

    fn facts(words: &[&fish_widestring::wstr]) -> FunctionHeaderFacts {
        FunctionHeaderFacts::of_header_words(words)
    }

    #[test]
    fn an_argument_names_option_declares_the_names_after_it() {
        let words = [L!("f"), L!("-a"), L!("a"), L!("b")];
        assert_eq!(
            facts(&words).argument_names,
            vec![L!("a").to_owned(), L!("b").to_owned()]
        );

        let words = [L!("f"), L!("--argument-names"), L!("a")];
        assert_eq!(facts(&words).argument_names, vec![L!("a").to_owned()]);

        // No argument-names option declares no name.
        assert!(facts(&[L!("f"), L!("b")]).argument_names.is_empty());
    }

    #[test]
    fn an_option_value_is_not_an_option() {
        // `--description -S` describes the function with the text `-S`.
        let words = [L!("f"), L!("--description"), L!("-S")];
        assert!(!facts(&words).no_scope_shadowing);
        assert!(facts(&words).argument_names.is_empty());

        // `-d -a` describes the function with the text `-a`.
        let words = [L!("f"), L!("-d"), L!("-a"), L!("a")];
        assert!(facts(&words).argument_names.is_empty());
    }

    #[test]
    fn the_no_shadowing_option_is_read_from_its_own_spelling() {
        assert!(
            facts(&[L!("f"), L!("-S")]).no_scope_shadowing,
            "the short spelling"
        );
        assert!(
            facts(&[L!("f"), L!("--no-scope-shadowing")]).no_scope_shadowing,
            "the long spelling"
        );
        assert!(
            !facts(&[L!("f")]).no_scope_shadowing,
            "a header without the option"
        );
    }

    #[test]
    fn a_name_after_another_option_is_no_name() {
        // The rule `function` applies: an option other than `--argument-names` ends the
        // run of names.
        let words = [L!("f"), L!("-a"), L!("a"), L!("-S"), L!("b")];
        assert_eq!(facts(&words).argument_names, vec![L!("a").to_owned()]);
        assert!(facts(&words).no_scope_shadowing);
    }
}
