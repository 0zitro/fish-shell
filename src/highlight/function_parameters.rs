//! The parameters of the function definition a character is inside.

use crate::ast::{self, Node as _};
use crate::prelude::*;
use fish_token_scope::TokenScopeId;
use fish_wcstringutil::string_prefixes_string;

/// The parameters one function definition declares.
///
/// The record is the names, `argv` being the one reserved name.  It is built from the
/// buffer being classified: the definition's own header names its parameters, so a
/// reference inside the body is classified without consulting the process.  Nested
/// definitions nest lists; a reference resolves against the innermost list alone.
pub struct FunctionParameterList {
    argument_names: Vec<WString>,
}

impl FunctionParameterList {
    /// The parameter list of one function definition.
    ///
    /// The names are the header's `-a` / `--argument-names` values and, after any such
    /// option, its remaining words.
    pub fn of_function_header(header: &ast::FunctionHeader, source: &wstr) -> Self {
        let mut words = vec![header.first_arg.source(source)];
        for argument in header.args.iter() {
            words.push(argument.source(source));
        }

        Self {
            argument_names: declared_argument_names(&words),
        }
    }

    /// The scope that classifies a reference to `name`, or none when the name is not a
    /// parameter of this definition.
    pub fn find_parameter_scope(&self, name: &wstr) -> Option<TokenScopeId> {
        if name == "argv" {
            return Some(TokenScopeId::VARIABLE_PARAMETER_ARGV);
        }
        if self
            .argument_names
            .iter()
            .any(|declared| declared.as_utfstr() == name)
        {
            return Some(TokenScopeId::VARIABLE_PARAMETER_NAMED);
        }
        None
    }
}

/// The names a function header's words declare.
///
/// The rule of `-a` and `--argument-names`: the option introduces names, the words after
/// it are names until an option is read, and a word that begins with `-` is an option.
/// The header's first word is the function's name, not a parameter.
fn declared_argument_names(words: &[&wstr]) -> Vec<WString> {
    let mut names = Vec::new();
    let mut declaring = false;

    for word in words.iter().skip(1) {
        if string_prefixes_string(L!("--argument-names="), word) {
            declaring = true;
            names.push(word[L!("--argument-names=").len()..].to_owned());
        } else if *word == "--argument-names" || *word == "-a" {
            declaring = true;
        } else if string_prefixes_string(L!("-a"), word) {
            // The option's value is attached to it, as in `-ax`.
            declaring = true;
            names.push(word[2..].to_owned());
        } else if word.starts_with('-') {
            declaring = false;
        } else if declaring {
            names.push((*word).to_owned());
        }
    }

    names
}

#[cfg(test)]
mod tests {
    use super::declared_argument_names;
    use crate::prelude::*;

    #[test]
    fn argument_names_follow_the_argument_names_option() {
        // No option declares nothing, and the first word is the function's name.
        assert!(declared_argument_names(&[L!("f")]).is_empty());
        assert!(declared_argument_names(&[L!("f"), L!("a"), L!("b")]).is_empty());

        // The option introduces the names that follow it.
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("-a"), L!("a"), L!("b")]),
            vec![L!("a").to_owned(), L!("b").to_owned()]
        );
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("--argument-names"), L!("a")]),
            vec![L!("a").to_owned()]
        );

        // The option's value may be attached to it.
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("-aa")]),
            vec![L!("a").to_owned()]
        );
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("--argument-names=a")]),
            vec![L!("a").to_owned()]
        );

        // Another option ends the run of names.
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("-a"), L!("a"), L!("-S"), L!("b")]),
            vec![L!("a").to_owned()]
        );
        assert_eq!(
            declared_argument_names(&[L!("f"), L!("-d"), L!("desc"), L!("-a"), L!("a")]),
            vec![L!("a").to_owned()]
        );
    }
}
