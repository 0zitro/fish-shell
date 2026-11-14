//! The parameters of the function definition a character is inside.

use crate::ast::{self, Node as _};
use crate::function_header::FunctionHeaderFacts;
use crate::prelude::*;
use fish_token_scope::TokenScopeId;

/// The parameters one function definition declares.
///
/// The record is the facts its header states, `argv` being the one reserved name.  It is
/// built from the buffer being classified: the definition's own header names its
/// parameters, so a reference inside the body is classified without consulting the
/// process.  Nested definitions nest lists; a reference resolves against the innermost
/// list alone.
pub struct FunctionParameterList {
    facts: FunctionHeaderFacts,
}

impl FunctionParameterList {
    /// The parameter list of one function definition.
    pub fn of_function_header(header: &ast::FunctionHeader, source: &wstr) -> Self {
        let mut words = vec![header.first_arg.source(source)];
        for argument in header.args.iter() {
            words.push(argument.source(source));
        }

        Self {
            facts: FunctionHeaderFacts::of_header_words(&words),
        }
    }

    /// Whether the definition's header declares that its body shares the calling scope.
    pub fn whether_no_scope_shadowing(&self) -> bool {
        self.facts.no_scope_shadowing
    }

    /// The scope that classifies a reference to `name`, or none when the name is not a
    /// parameter of this definition.
    pub fn find_parameter_scope(&self, name: &wstr) -> Option<TokenScopeId> {
        if name == "argv" {
            return Some(TokenScopeId::VARIABLE_PARAMETER_ARGV);
        }
        if self
            .facts
            .argument_names
            .iter()
            .any(|declared| declared.as_utfstr() == name)
        {
            return Some(TokenScopeId::VARIABLE_PARAMETER_NAMED);
        }
        None
    }
}
