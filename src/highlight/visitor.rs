//! Classification of shell source into token scopes.
use crate::{
    abbrs::{self, with_abbrs},
    ast::{
        self, Argument, BlockStatement, BlockStatementHeader, BraceStatement, DecoratedStatement,
        Keyword, Kind, Node, NodeVisitor, Redirection, Token, VariableAssignment,
    },
    builtins::builtin_exists,
    common::{valid_var_name, valid_var_name_char},
    complete::complete_wrap_map,
    env::{ENV_MODE_FLAGS, EnvMode, Environment, VariableScope},
    expand::{ExpandFlags, ExpandResultCode, expand_one, expand_to_command_and_args},
    function,
    highlight::HighlightClassification,
    highlight::file_tester::FileTester,
    highlight::style_resolution::HighlightStyleResolver,
    history::all_paths_are_valid,
    operation_context::OperationContext,
    parse_constants::{
        ParseKeyword, ParseTokenType, ParseTreeFlags, SourceRange, StatementDecoration,
    },
    parse_util::{get_process_first_token_offset, locate_cmdsubst_range, slice_length},
    parser_keywords::parser_keywords_is_reserved,
    path::{path_as_implicit_cd, path_get_cdpath, path_get_path, paths_are_same_file},
    redirection::RedirectionMode,
    terminal::Outputter,
    text_style::TextStyle,
    threads::assert_is_background_thread,
    tokenizer::{PipeOrRedir, variable_assignment_equals_pos},
};
use fish_color::Color;
use fish_feature_flags::{FeatureFlag, feature_test};
use fish_token_scope::{
    EnclosingScopeSet, HighlightAttribute, TokenScopeId, builtin_word_scope, keyword_word_scope,
};
use fish_wcstringutil::string_prefixes_string;
use fish_widestring::{
    ASCII_MAX, EXPAND_RESERVED_BASE, EXPAND_RESERVED_END, L, PROCESS_EXPAND_SELF_STR, WExt as _,
    WString, wstr,
};

use super::file_tester::IsFile;
use super::function_parameters::FunctionParameterList;

/// Writes `text` with each character's resolved style as ANSI escape sequences.
pub fn colorize_text(
    text: &wstr,
    classifications: &[HighlightClassification],
    vars: &dyn Environment,
) -> Vec<u8> {
    assert_eq!(classifications.len(), text.len());
    let mut rv = HighlightStyleResolver::new();
    let mut outp = Outputter::new_buffering();

    let mut last_classification = HighlightClassification::default();
    for (i, c) in text.chars().enumerate() {
        let classification = classifications[i];
        if classification != last_classification {
            let mut style = rv.style_of(classification, vars);
            style.bg = Color::Normal; // Historical behavior.
            outp.set_text_style(style);
            last_classification = classification;
        }
        if i + 1 == text.char_count() && c == '\n' {
            outp.set_text_style(TextStyle::terminal_default());
        }
        outp.writech(c);
    }
    outp.set_text_style(TextStyle::terminal_default());
    outp.contents().to_owned()
}

/// Perform syntax highlighting for the shell commands in buff. The result is stored in the color
/// array as a color_code from the HIGHLIGHT_ enum for each character in buff.
///
/// \param buffstr The buffer on which to perform syntax highlighting
/// \param classifications The array in which to store one classification per character.
/// \param ctx The variables and cancellation check for this operation.
/// \param io_ok If set, allow IO which may block. This means that e.g. invalid commands may be
/// detected.
/// \param cursor The position of the cursor in the commandline.
pub fn highlight_shell<'src, 'ctx>(
    buff: &'src wstr,
    classifications: &mut Vec<HighlightClassification>,
    ctx: &'ctx mut OperationContext<'src>,
    io_ok: bool, /* = false */
    cursor: Option<usize>,
) {
    let working_directory = ctx.vars().get_pwd_slash();
    let mut visitor = HighlightVisitor::new(buff, cursor, ctx, &working_directory, io_ok);
    *classifications = visitor.classify();
}

pub fn highlight_and_colorize<'src, 'ctx>(
    text: &'src wstr,
    ctx: &'ctx mut OperationContext<'src>,
) -> Vec<u8> {
    let mut classifications = Vec::new();
    highlight_shell(
        text,
        &mut classifications,
        ctx,
        /*io_ok=*/ false,
        /*cursor=*/ None,
    );
    let vars = ctx.vars();
    colorize_text(text, &classifications, vars)
}

fn command_scope(
    cmd: &wstr,
    decoration: StatementDecoration,
    working_directory: &wstr,
    vars: &dyn Environment,
) -> Option<TokenScopeId> {
    // Determine which types we check, based on the decoration.
    let mut builtin_ok = true;
    let mut function_ok = true;
    let mut abbreviation_ok = true;
    let mut command_ok = true;
    let mut implicit_cd_ok = true;
    if matches!(
        decoration,
        StatementDecoration::Command | StatementDecoration::Exec
    ) {
        builtin_ok = false;
        function_ok = false;
        abbreviation_ok = false;
        command_ok = true;
        implicit_cd_ok = false;
    } else if decoration == StatementDecoration::Builtin {
        builtin_ok = true;
        function_ok = false;
        abbreviation_ok = false;
        command_ok = false;
        implicit_cd_ok = false;
    }

    // A function takes the name it declares, and a function of a builtin's name is what the
    // word runs, so the function is consulted first.
    // Functions
    if function_ok && function::exists_no_autoload(cmd) {
        return Some(TokenScopeId::FUNCTION);
    }

    // Builtins
    if builtin_ok && builtin_exists(cmd) {
        return Some(builtin_word_scope(cmd).unwrap_or(TokenScopeId::BUILTIN));
    }

    // Abbreviations
    if abbreviation_ok && with_abbrs(|set| set.has_match(cmd, abbrs::Position::Command, L!(""))) {
        return Some(TokenScopeId::COMMAND_ABBREVIATION);
    }

    // Regular commands
    if command_ok && path_get_path(cmd, vars).is_some() {
        return Some(TokenScopeId::COMMAND_EXTERNAL);
    }

    // Implicit cd
    if implicit_cd_ok && path_as_implicit_cd(cmd, working_directory, vars).is_some() {
        return Some(TokenScopeId::COMMAND_IMPLICIT_CD);
    }

    None
}

fn has_expand_reserved(s: &wstr) -> bool {
    for wc in s.chars() {
        if (EXPAND_RESERVED_BASE..=EXPAND_RESERVED_END).contains(&wc) {
            return true;
        }
    }
    false
}

// Parse a command line. Return by reference the first command, and the first argument to that
// command (as a string), if any. This is used to validate autosuggestions.
fn autosuggest_parse_command(
    buff: &wstr,
    ctx: &mut OperationContext<'_>,
) -> Option<(WString, WString)> {
    let flags = ParseTreeFlags {
        continue_after_error: true,
        accept_incomplete_tokens: true,
        ..Default::default()
    };
    let ast = ast::parse(buff, flags, None);

    // Find the first statement.
    let job_list: &ast::JobList = ast.top();
    let jc = job_list.first()?;
    let first_statement = jc.job.statement.as_decorated_statement()?;

    if let Some(expanded_command) = statement_get_expanded_command(buff, first_statement, ctx) {
        let mut arg = WString::new();
        // Check if the first argument or redirection is, in fact, an argument.
        if let Some(arg_or_redir) = first_statement.args_or_redirs.first() {
            if arg_or_redir.is_argument() {
                arg = arg_or_redir.argument().source(buff).to_owned();
            }
        }

        return Some((expanded_command, arg));
    }

    None
}

pub fn is_veritable_cd(expanded_command: &wstr) -> bool {
    expanded_command == L!("cd") && complete_wrap_map().get(L!("cd")).is_none()
}

fn is_cd_option(candidate: &wstr) -> bool {
    let args = vec![
        L!("--help"),
        L!("-h"),
        L!("--logical"),
        L!("-L"),
        L!("--physical"),
        L!("-P"),
    ];
    args.into_iter()
        .any(|arg| string_prefixes_string(candidate, arg))
}

/// Given an item `item` from the history which is a proposed autosuggestion, return whether the
/// autosuggestion is valid. It may not be valid if e.g. it is attempting to cd into a directory
/// which does not exist.
pub fn autosuggest_validate_from_history(
    item_commandline: &wstr,
    suggested_range: std::ops::Range<usize>,
    required_paths: &[WString],
    working_directory: &wstr,
    ctx: &mut OperationContext<'_>,
) -> bool {
    assert_is_background_thread();

    if suggested_range != (0..item_commandline.char_count())
        && get_process_first_token_offset(item_commandline, suggested_range.start)
            .is_some_and(|offset| offset != suggested_range.start)
    {
        return false;
    }

    // Parse the string.
    let suggested_command = &item_commandline[suggested_range];
    let Some((parsed_command, mut cd_dir)) = autosuggest_parse_command(suggested_command, ctx)
    else {
        // This is for autosuggestions which are not decorated commands, e.g. function declarations.
        return true;
    };

    // We handle cd specially.
    if is_veritable_cd(&parsed_command)
        && !cd_dir.is_empty()
        && expand_one(&mut cd_dir, ExpandFlags::FAIL_ON_CMDSUBST, ctx, None)
    {
        if is_cd_option(&cd_dir) {
            // known arguments are always valid
            return true;
        } else {
            // Check the directory target, respecting CDPATH.
            // Permit the autosuggestion if the path is valid and not our directory.
            let path = path_get_cdpath(&cd_dir, working_directory, ctx.vars());
            return path.is_some_and(|p| !paths_are_same_file(working_directory, &p));
        }
    }

    // Not handled specially. Is the command valid?
    let cmd_ok = builtin_exists(&parsed_command)
        || function::exists_no_autoload(&parsed_command)
        || path_get_path(&parsed_command, ctx.vars()).is_some();
    if !cmd_ok {
        return false;
    }

    // Did the historical command have arguments that look like paths, which aren't paths now?
    if !all_paths_are_valid(required_paths, ctx) {
        return false;
    }

    true
}

// Classifies the variable reference starting with `inp`'s first `$`, writing one
// classification per character into `classifications`. Returns the number of characters
// consumed.
fn classify_variable(
    inp: &wstr,
    parameters: Option<&FunctionParameterList>,
    classifications: &mut [HighlightClassification],
) -> usize {
    assert_eq!(inp.char_at(0), '$');
    let sigil = HighlightClassification::from(TokenScopeId::VARIABLE_SIGIL);
    let dereference = HighlightClassification::from(TokenScopeId::VARIABLE_SIGIL_DEREFERENCE);
    let variable = HighlightClassification::from(TokenScopeId::VARIABLE);
    let slice = HighlightClassification::from(TokenScopeId::VARIABLE_SLICE);
    let dollar = HighlightClassification::from(TokenScopeId::OPERATOR_DOLLAR);
    let invalid = HighlightClassification::from(TokenScopeId::ERROR);

    // The run of dollars: the one adjacent to what the reference reads is the sigil, and
    // the ones before it dereference it.
    let mut dollar_count = 0;
    while inp.char_at(dollar_count) == '$' {
        dollar_count += 1;
    }
    let next = inp.char_at(dollar_count);

    if next == '(' {
        // A command substitution, or a dynamic reference whose name the substitution
        // produces; the parenthesis is the substitution's own operator.
        if dollar_count == 1 {
            classifications[0] = dollar;
            return 1;
        }
        classifications[..dollar_count - 1].fill(dereference);
        classifications[dollar_count - 1] = sigil;
        return dollar_count;
    }

    classifications[..dollar_count - 1].fill(dereference);
    classifications[dollar_count - 1] = if valid_var_name_char(next) {
        sigil
    } else {
        invalid
    };
    let mut idx = dollar_count;

    // Handle a sequence of variable characters.
    // It may contain an escaped newline - see #8444.
    let name_start = idx;
    loop {
        if valid_var_name_char(inp.char_at(idx)) {
            idx += 1;
        } else if inp.char_at(idx) == '\\' && inp.char_at(idx + 1) == '\n' {
            idx += 2;
        } else {
            break;
        }
    }
    // A reference to a parameter of the enclosing definition is classified by it.
    let name = &inp[name_start..idx];
    let name_classification = parameters
        .and_then(|parameters| parameters.find_parameter_scope(name))
        .map_or(variable, HighlightClassification::from);
    classifications[name_start..idx].fill(name_classification);

    // Handle a slice, up to dollar_count of them. Note that we currently don't do any validation of
    // the slice's contents, e.g. $foo[blah] will not show an error even though it's invalid.
    for _slice_count in 0..dollar_count {
        match slice_length(&inp[idx..]) {
            Some(slice_len) if slice_len > 0 => {
                classifications[idx] = slice;
                classifications[idx + slice_len - 1] = slice;
                idx += slice_len;
            }
            Some(_slice_len) => {
                // not a slice
                break;
            }
            None => {
                // Syntax error. Normally the entire token is colored red for us, but inside a
                // double-quoted string that doesn't happen. As such, color the variable + the slice
                // start red. Coloring any more than that looks bad, unless we're willing to try and
                // detect where the double-quoted string ends, and I'd rather not do that.
                classifications[..=idx].fill(invalid);
                break;
            }
        }
    }
    idx
}

/// This function is a disaster badly in need of refactoring. It colors an argument or command,
/// without regard to command substitutions.
fn classify_word(
    buffstr: &wstr,
    parameters: Option<&FunctionParameterList>,
    base: HighlightClassification,
    classifications: &mut [HighlightClassification],
) {
    let operator = HighlightClassification::from(TokenScopeId::OPERATOR);
    let invalid = HighlightClassification::from(TokenScopeId::ERROR);
    let escape = HighlightClassification::from(TokenScopeId::ESCAPE);
    let glob = HighlightClassification::from(TokenScopeId::OPERATOR_GLOB);
    let brace = HighlightClassification::from(TokenScopeId::OPERATOR_BRACE);
    let parenthesis = HighlightClassification::from(TokenScopeId::OPERATOR_PARENTHESIS);
    let tilde = HighlightClassification::from(TokenScopeId::OPERATOR_TILDE);
    let delimiter_single = HighlightClassification::from(TokenScopeId::QUOTE_SINGLE_DELIMITER);
    let delimiter_double = HighlightClassification::from(TokenScopeId::QUOTE_DOUBLE_DELIMITER);
    let quoted_single = HighlightClassification::from(TokenScopeId::QUOTE_SINGLE);
    let quoted_double = HighlightClassification::from(TokenScopeId::QUOTE_DOUBLE);
    let buff_len = buffstr.len();
    classifications.fill(base);

    // Hacky support for %self which must be an unquoted literal argument.
    if buffstr == PROCESS_EXPAND_SELF_STR {
        classifications[..PROCESS_EXPAND_SELF_STR.len()].fill(operator);
        return;
    }

    #[derive(PartialEq)]
    enum Mode {
        Unquoted,
        SingleQuoted,
        DoubleQuoted,
    }
    let mut mode = Mode::Unquoted;
    let mut unclosed_quote_offset = None;
    let mut bracket_count = 0;
    let mut in_pos = 0;
    while in_pos < buff_len {
        let c = buffstr.as_char_slice()[in_pos];
        match mode {
            Mode::Unquoted => {
                if c == '\\' {
                    let mut fill_scope = TokenScopeId::ESCAPE; // may become another class
                    let backslash_pos = in_pos;
                    let mut fill_end = backslash_pos;

                    // Move to the escaped character.
                    in_pos += 1;
                    let escaped_char = if in_pos < buff_len {
                        buffstr.as_char_slice()[in_pos]
                    } else {
                        '\0'
                    };

                    if escaped_char == '\0' {
                        fill_end = in_pos;
                        fill_scope = TokenScopeId::ERROR;
                    } else if matches!(escaped_char, '~' | '%') {
                        if in_pos == 1 {
                            fill_end = in_pos + 1;
                        }
                    } else if escaped_char == ',' {
                        if bracket_count != 0 {
                            fill_end = in_pos + 1;
                        }
                    } else if "abefnrtv*?$(){}[]'\"<>^ \\#;|&".contains(escaped_char) {
                        fill_end = in_pos + 1;
                    } else if escaped_char == 'c' {
                        // Like \ci. So highlight three characters.
                        fill_scope = TokenScopeId::ESCAPE_CONTROL;
                        fill_end = in_pos + 1;
                    } else if "uUxX01234567".contains(escaped_char) {
                        let mut res = 0;
                        let mut chars = 2;
                        let mut base = 16;
                        let mut max_val = ASCII_MAX;

                        match escaped_char {
                            'u' => {
                                chars = 4;
                                max_val = '\u{FFFF}'; // UCS2_MAX
                                in_pos += 1;
                            }
                            'U' => {
                                chars = 8;
                                // Don't exceed the largest Unicode code point - see #1107.
                                max_val = '\u{10FFFF}';
                                in_pos += 1;
                            }
                            'x' | 'X' => {
                                max_val = 0xFF as char;
                                in_pos += 1;
                            }
                            _ => {
                                // a digit like \12
                                base = 8;
                                chars = 3;
                            }
                        }

                        // Consume
                        for _i in 0..chars {
                            if in_pos == buff_len {
                                break;
                            }
                            let Some(d) = buffstr.as_char_slice()[in_pos].to_digit(base) else {
                                break;
                            };
                            res = (res * base) + d;
                            in_pos += 1;
                        }
                        // in_pos is now at the first character that could not be converted (or
                        // buff_len).
                        assert!((backslash_pos..=buff_len).contains(&in_pos));
                        fill_end = in_pos;

                        // It's an error if we exceeded the max value.
                        if res > u32::from(max_val) {
                            fill_scope = TokenScopeId::ESCAPE_INVALID;
                        } else {
                            fill_scope = if base == 8 {
                                TokenScopeId::ESCAPE_OCTAL
                            } else {
                                TokenScopeId::ESCAPE_UNICODE
                            };
                        }

                        // Subtract one from in_pos, so that the increment in the loop will move to
                        // the next character.
                        in_pos -= 1;
                    }
                    assert!(fill_end >= backslash_pos);
                    classifications[backslash_pos..fill_end]
                        .fill(HighlightClassification::from(fill_scope));
                } else {
                    // Not a backslash.
                    match c {
                        '~' if in_pos == 0 => {
                            classifications[in_pos] = tilde;
                        }
                        '$' => {
                            assert!(in_pos < buff_len);
                            in_pos += classify_variable(
                                &buffstr[in_pos..],
                                parameters,
                                &mut classifications[in_pos..],
                            );
                            // Subtract one to account for the upcoming loop increment.
                            in_pos -= 1;
                        }
                        '?' if !feature_test(FeatureFlag::QuestionMarkNoGlob) => {
                            classifications[in_pos] = glob;
                        }
                        '*' => {
                            classifications[in_pos] = glob;
                        }
                        '(' | ')' => {
                            classifications[in_pos] = parenthesis;
                        }
                        '{' => {
                            classifications[in_pos] = brace;
                            bracket_count += 1;
                        }
                        '}' => {
                            classifications[in_pos] = brace;
                            bracket_count -= 1;
                        }
                        ',' if bracket_count > 0 => {
                            classifications[in_pos] = brace;
                        }
                        '\'' => {
                            classifications[in_pos] = delimiter_single;
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::SingleQuoted;
                        }
                        '"' => {
                            classifications[in_pos] = delimiter_double;
                            unclosed_quote_offset = Some(in_pos);
                            mode = Mode::DoubleQuoted;
                        }
                        _ => (), // we ignore all other characters
                    }
                }
            }
            // Mode 1 means single quoted string, i.e 'foo'.
            Mode::SingleQuoted => {
                classifications[in_pos] = quoted_single;
                if c == '\\' {
                    // backslash
                    if in_pos + 1 < buff_len {
                        let escaped_char = buffstr.as_char_slice()[in_pos + 1];
                        if matches!(escaped_char, '\\' | '\'') {
                            classifications[in_pos] = escape; // backslash
                            classifications[in_pos + 1] = escape; // escaped char
                            in_pos += 1; // skip over backslash
                        }
                    }
                } else if c == '\'' {
                    classifications[in_pos] = delimiter_single;
                    mode = Mode::Unquoted;
                }
            }
            // Mode 2 means double quoted string, i.e. "foo".
            Mode::DoubleQuoted => {
                // Slices are colored in advance, past `in_pos`, and we don't want to overwrite
                // that.
                if classifications[in_pos] == base {
                    classifications[in_pos] = quoted_double;
                }
                match c {
                    '"' => {
                        classifications[in_pos] = delimiter_double;
                        mode = Mode::Unquoted;
                    }
                    '\\'
                        // Backslash
                        if in_pos + 1 < buff_len => {
                            let escaped_char = buffstr.as_char_slice()[in_pos + 1];
                            if matches!(escaped_char, '\\' | '"' | '\n' | '$') {
                                classifications[in_pos] = escape; // backslash
                                classifications[in_pos + 1] = escape; // escaped char
                                in_pos += 1; // skip over backslash
                            }
                        }
                    '$' => {
                        in_pos += classify_variable(
                            &buffstr[in_pos..],
                            parameters,
                            &mut classifications[in_pos..],
                        );
                        // Subtract one to account for the upcoming increment in the loop.
                        in_pos -= 1;
                    }
                    _ => (), // we ignore all other characters
                }
            }
        }
        in_pos += 1;
    }

    // Error on unclosed quotes.
    if mode != Mode::Unquoted {
        classifications[unclosed_quote_offset.unwrap()] = invalid;
    }
}

/// How a word's shape spells an option.
enum OptionShape {
    /// The word is not an option.
    Argument,
    /// The whole word is one option shape.
    Whole(TokenScopeId),
    /// A long option whose `=` splits the name from its value; the value is the index of
    /// that `=`.
    LongWithValue { name_end: usize },
}

/// The option shape a word has, when options are allowed here.
///
/// The shapes are spellings, not readings: a single dash with several characters is one
/// shape whether it is a GNU-style long option or a bundle of short ones, because telling
/// them apart needs the command's own option table, which the classifier does not read.
fn option_shape(word: &wstr, options_allowed: bool) -> OptionShape {
    if !options_allowed || word.is_empty() || word.char_at(0) != '-' {
        return OptionShape::Argument;
    }

    // `-` and `--` are themselves options.
    if word.len() == 1 {
        return OptionShape::Whole(TokenScopeId::OPTION);
    }

    if word.char_at(1) == '-' {
        if word.len() == 2 {
            return OptionShape::Whole(TokenScopeId::OPTION);
        }
        // A long option, which splits at its `=` if it has one.
        let mut index = 2;
        while index < word.len() {
            if word.char_at(index) == '=' {
                return OptionShape::LongWithValue { name_end: index };
            }
            index += 1;
        }
        return OptionShape::Whole(TokenScopeId::OPTION_LONG);
    }

    let rest = &word[1..];
    if rest.chars().all(|character| character.is_ascii_digit()) {
        // A negative number is a value, not an option.
        OptionShape::Whole(TokenScopeId::OPTION_NUMERIC)
    } else if rest.len() == 1 {
        OptionShape::Whole(TokenScopeId::OPTION_SHORT)
    } else {
        OptionShape::Whole(TokenScopeId::OPTION_GNU)
    }
}

/// The enclosing set that holds this one construct scope.
fn enclosing_of(scope: TokenScopeId) -> EnclosingScopeSet {
    EnclosingScopeSet::default().with_scope(scope)
}

/// The enclosing set that holds both of these construct scopes.
fn enclosing_of_both(first: TokenScopeId, second: TokenScopeId) -> EnclosingScopeSet {
    enclosing_of(first).with_scope(second)
}

/// The enclosing set a block statement's body is inside, by the header that opens it.
fn body_enclosing(
    header: &BlockStatementHeader,
    function_parameters: Option<&FunctionParameterList>,
) -> EnclosingScopeSet {
    match header {
        BlockStatementHeader::Function(_) => {
            let mut scopes = enclosing_of(TokenScopeId::META_FUNCTION);
            if function_parameters.is_some_and(|parameters| parameters.whether_no_scope_shadowing())
            {
                scopes = scopes.with_scope(TokenScopeId::META_FUNCTION_NO_SHADOWING);
            }
            scopes
        }
        BlockStatementHeader::Begin(_) => enclosing_of(TokenScopeId::META_BLOCK),
        BlockStatementHeader::For(_) => {
            enclosing_of_both(TokenScopeId::META_LOOP, TokenScopeId::META_LOOP_FOR)
        }
        BlockStatementHeader::While(_) => {
            enclosing_of_both(TokenScopeId::META_LOOP, TokenScopeId::META_LOOP_WHILE)
        }
    }
}

/// The scope that classifies a variable name written in this definition mode.
///
/// The projection from the environment's domain into the vocabulary's: lossy, so it is a
/// statement rather than a conversion, and it lives here rather than in either domain
/// because `EnvMode` must not learn the vocabulary.  Each factor contributes its value's
/// path component unless the mode leaves the factor absent.
pub fn definition_scope(mode: &EnvMode) -> TokenScopeId {
    use TokenScopeId as Scope;

    let export = mode.export;
    let pathvar = mode.pathvar;
    match mode.write_scope() {
        None => match (export, pathvar) {
            (None, None) => Scope::VARIABLE_DEFINITION,
            (None, Some(true)) => Scope::VARIABLE_DEFINITION_PATH,
            (None, Some(false)) => Scope::VARIABLE_DEFINITION_NON_PATH,
            (Some(true), None) => Scope::VARIABLE_DEFINITION_EXPORTED,
            (Some(false), None) => Scope::VARIABLE_DEFINITION_UNEXPORTED,
            (Some(true), Some(true)) => Scope::VARIABLE_DEFINITION_EXPORTED_PATH,
            (Some(true), Some(false)) => Scope::VARIABLE_DEFINITION_EXPORTED_NON_PATH,
            (Some(false), Some(true)) => Scope::VARIABLE_DEFINITION_UNEXPORTED_PATH,
            (Some(false), Some(false)) => Scope::VARIABLE_DEFINITION_UNEXPORTED_NON_PATH,
        },
        Some(VariableScope::Local) => match (export, pathvar) {
            (None, None) => Scope::VARIABLE_DEFINITION_LOCAL,
            (None, Some(true)) => Scope::VARIABLE_DEFINITION_LOCAL_PATH,
            (None, Some(false)) => Scope::VARIABLE_DEFINITION_LOCAL_NON_PATH,
            (Some(true), None) => Scope::VARIABLE_DEFINITION_LOCAL_EXPORTED,
            (Some(false), None) => Scope::VARIABLE_DEFINITION_LOCAL_UNEXPORTED,
            (Some(true), Some(true)) => Scope::VARIABLE_DEFINITION_LOCAL_EXPORTED_PATH,
            (Some(true), Some(false)) => Scope::VARIABLE_DEFINITION_LOCAL_EXPORTED_NON_PATH,
            (Some(false), Some(true)) => Scope::VARIABLE_DEFINITION_LOCAL_UNEXPORTED_PATH,
            (Some(false), Some(false)) => Scope::VARIABLE_DEFINITION_LOCAL_UNEXPORTED_NON_PATH,
        },
        Some(VariableScope::Function) => match (export, pathvar) {
            (None, None) => Scope::VARIABLE_DEFINITION_FUNCTION,
            (None, Some(true)) => Scope::VARIABLE_DEFINITION_FUNCTION_PATH,
            (None, Some(false)) => Scope::VARIABLE_DEFINITION_FUNCTION_NON_PATH,
            (Some(true), None) => Scope::VARIABLE_DEFINITION_FUNCTION_EXPORTED,
            (Some(false), None) => Scope::VARIABLE_DEFINITION_FUNCTION_UNEXPORTED,
            (Some(true), Some(true)) => Scope::VARIABLE_DEFINITION_FUNCTION_EXPORTED_PATH,
            (Some(true), Some(false)) => Scope::VARIABLE_DEFINITION_FUNCTION_EXPORTED_NON_PATH,
            (Some(false), Some(true)) => Scope::VARIABLE_DEFINITION_FUNCTION_UNEXPORTED_PATH,
            (Some(false), Some(false)) => Scope::VARIABLE_DEFINITION_FUNCTION_UNEXPORTED_NON_PATH,
        },
        Some(VariableScope::Global) => match (export, pathvar) {
            (None, None) => Scope::VARIABLE_DEFINITION_GLOBAL,
            (None, Some(true)) => Scope::VARIABLE_DEFINITION_GLOBAL_PATH,
            (None, Some(false)) => Scope::VARIABLE_DEFINITION_GLOBAL_NON_PATH,
            (Some(true), None) => Scope::VARIABLE_DEFINITION_GLOBAL_EXPORTED,
            (Some(false), None) => Scope::VARIABLE_DEFINITION_GLOBAL_UNEXPORTED,
            (Some(true), Some(true)) => Scope::VARIABLE_DEFINITION_GLOBAL_EXPORTED_PATH,
            (Some(true), Some(false)) => Scope::VARIABLE_DEFINITION_GLOBAL_EXPORTED_NON_PATH,
            (Some(false), Some(true)) => Scope::VARIABLE_DEFINITION_GLOBAL_UNEXPORTED_PATH,
            (Some(false), Some(false)) => Scope::VARIABLE_DEFINITION_GLOBAL_UNEXPORTED_NON_PATH,
        },
        Some(VariableScope::Universal) => match (export, pathvar) {
            (None, None) => Scope::VARIABLE_DEFINITION_UNIVERSAL,
            (None, Some(true)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_PATH,
            (None, Some(false)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_NON_PATH,
            (Some(true), None) => Scope::VARIABLE_DEFINITION_UNIVERSAL_EXPORTED,
            (Some(false), None) => Scope::VARIABLE_DEFINITION_UNIVERSAL_UNEXPORTED,
            (Some(true), Some(true)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_EXPORTED_PATH,
            (Some(true), Some(false)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_EXPORTED_NON_PATH,
            (Some(false), Some(true)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_UNEXPORTED_PATH,
            (Some(false), Some(false)) => Scope::VARIABLE_DEFINITION_UNIVERSAL_UNEXPORTED_NON_PATH,
        },
    }
}

/// Which words of a definition command are the names it defines.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum DefinitionCommand {
    /// `set` defines its first name; the words after it are that name's values.
    Set,
    /// `read` defines every name it is given.
    Read,
}

impl DefinitionCommand {
    /// The definition command `word` names, if it names one.
    fn of_command(word: &wstr) -> Option<Self> {
        if word == "set" {
            Some(Self::Set)
        } else if word == "read" {
            Some(Self::Read)
        } else {
            None
        }
    }

    /// Whether every name the command is given is defined, rather than only the first.
    fn whether_defines_each_name(self) -> bool {
        self == Self::Read
    }
}

/// The mode a definition command's arguments request, or none when they carry an option
/// the classifier does not read.
///
/// The flags are read against the one declaration the definition commands share.  A word
/// that is no flag ends the scan, which is exactly the order `set` reads its arguments in,
/// and is the conservative reading for `read`: its names are classified with the mode the
/// leading flags request.
fn definition_mode<'a>(
    arguments: impl Iterator<Item = &'a ast::ArgumentOrRedirection>,
    source: &wstr,
) -> Option<EnvMode> {
    let mut mode = EnvMode::default();
    let mut reading_flags = true;
    for argument_or_redirection in arguments {
        let ast::ArgumentOrRedirection::Argument(argument) = argument_or_redirection else {
            continue;
        };
        let word = argument.source(source);
        if !reading_flags {
            continue;
        }
        if word == "--" {
            reading_flags = false;
            continue;
        }
        if !word.starts_with('-') || word == "-" {
            reading_flags = false;
            continue;
        }
        let flag = ENV_MODE_FLAGS
            .iter()
            .find(|flag| flag.whether_spelled_by(word));
        if let Some(flag) = flag {
            flag.sets.set(&mut mode);
            continue;
        }

        // A short option may bundle several flags, and none of them takes an argument.
        if word.char_at(1) == '-' || word.len() <= 2 {
            return None;
        }
        for index in 1..word.len() {
            let flag = ENV_MODE_FLAGS
                .iter()
                .find(|flag| flag.short == word.char_at(index))?;
            flag.sets.set(&mut mode);
        }
    }
    Some(mode)
}

/// The scope a redirection's mode denotes.
fn redirection_mode_scope(mode: RedirectionMode) -> TokenScopeId {
    match mode {
        RedirectionMode::Input | RedirectionMode::TryInput => TokenScopeId::REDIRECTION_INPUT,
        RedirectionMode::Overwrite | RedirectionMode::NoClob => TokenScopeId::REDIRECTION_OUTPUT,
        RedirectionMode::Append => TokenScopeId::REDIRECTION_APPEND,
        RedirectionMode::Fd => TokenScopeId::REDIRECTION_FD,
    }
}

/// The parse tree's visitor: every character is classified as the traversal
/// reaches it.
struct HighlightVisitor<'src, 'wd, 'ctx> {
    // The string we're highlighting. Note this is a reference member variable (to avoid copying)!
    buff: &'src wstr,
    // The position of the cursor within the string.
    cursor: Option<usize>,
    // Whether it's OK to do I/O.
    io_ok: bool,
    // Working directory.
    working_directory: &'wd wstr,
    // Our component for testing strings for being potential file paths.
    file_tester: FileTester<'src, 'wd, 'ctx>,
    // The classification of every character, in buffer order.
    classifications: Vec<HighlightClassification>,
    // A stack of variables that the current commandline probably defines.  We mark redirections
    // as valid if they use one of these variables, to avoid marking valid targets as error.
    pending_variable_names: Vec<&'src wstr>,
    /// The parameter lists of the function definitions this buffer is inside, innermost
    /// last.
    enclosing_function_parameters: Vec<FunctionParameterList>,
    /// The enclosing scopes of the region the traversal is inside, inherited by the
    /// buffers this visit descends into.
    enclosing_scopes: EnclosingScopeSet,
    done: bool,
}

impl<'src, 'wd, 'ctx> HighlightVisitor<'src, 'wd, 'ctx> {
    pub fn new(
        buff: &'src wstr,
        cursor: Option<usize>,
        ctx: &'ctx mut OperationContext<'src>,
        working_directory: &'wd wstr,
        can_do_io: bool,
    ) -> Self {
        Self::inside(
            buff,
            cursor,
            ctx,
            working_directory,
            can_do_io,
            EnclosingScopeSet::default(),
        )
    }

    /// A visitor for a region that lies inside `inherited`: the enclosing scopes of
    /// the buffer this visit descends from become every character's.
    fn inside(
        buff: &'src wstr,
        cursor: Option<usize>,
        ctx: &'ctx mut OperationContext<'src>,
        working_directory: &'wd wstr,
        can_do_io: bool,
        inherited: EnclosingScopeSet,
    ) -> Self {
        let file_tester = FileTester::new(working_directory, ctx);
        Self {
            buff,
            cursor,
            io_ok: can_do_io,
            working_directory,
            file_tester,
            classifications: vec![],
            pending_variable_names: vec![],
            enclosing_function_parameters: vec![],
            enclosing_scopes: inherited,
            done: false,
        }
    }

    fn ctx(&self) -> &OperationContext<'src> {
        self.file_tester.ctx
    }

    /// The classification of every character of the buffer, in buffer order.
    pub fn classify(&mut self) -> Vec<HighlightClassification> {
        assert!(!self.done);
        self.done = true;

        // If we are doing I/O, we must be in a background thread.
        if self.io_ok {
            assert_is_background_thread();
        }

        self.classifications
            .resize(self.buff.len(), HighlightClassification::default());

        // Flags we use for AST parsing.
        let ast_flags = ParseTreeFlags {
            continue_after_error: true,
            include_comments: true,
            accept_incomplete_tokens: true,
            leave_unterminated: true,
            show_extra_semis: true,
            ..Default::default()
        };
        let ast = ast::parse(self.buff, ast_flags, None);

        self.visit_children(ast.top());
        if self.file_tester.ctx.check_cancel() {
            return std::mem::take(&mut self.classifications);
        }

        // Classify every comment.
        let extras = &ast.extras;
        for range in &extras.comments {
            self.classify_span(*range, self.classification_of(TokenScopeId::COMMENT));
        }

        // Classify every extra semi.
        for range in &extras.semis {
            self.classify_terminator_span(*range);
        }

        // Classify every error range.
        for range in &extras.errors {
            self.classify_span(*range, self.classification_of(TokenScopeId::ERROR));
        }

        std::mem::take(&mut self.classifications)
    }

    /// Return a substring of our buffer.
    pub fn source_slice(&self, r: SourceRange) -> &'src wstr {
        assert!(r.end() >= r.start(), "Overflow");
        assert!(r.end() <= self.buff.len(), "Out of range");
        &self.buff[r.start()..r.end()]
    }

    fn whether_io_permitted(&self) -> bool {
        self.io_ok && !self.ctx().check_cancel()
    }

    /// The classification one scope denotes.
    fn classification_of(&self, scope: TokenScopeId) -> HighlightClassification {
        HighlightClassification::from(scope).with_enclosing(self.enclosing_scopes)
    }

    /// Visit a region whose characters are inside `scopes` in addition to the ones the
    /// traversal is already inside.
    fn visit_inside(&mut self, scopes: EnclosingScopeSet, region: impl FnOnce(&mut Self)) {
        let enclosing = self.enclosing_scopes;
        self.enclosing_scopes = enclosing | scopes;
        region(self);
        self.enclosing_scopes = enclosing;
    }

    // Classify a command word.
    fn classify_command_word(&mut self, node: &ast::String_, scope: TokenScopeId) {
        let source_range = node.source_range();
        let cmd_str = self.source_slice(source_range);
        let base = self.classification_of(scope);

        classify_word(
            cmd_str,
            self.enclosing_function_parameters.last(),
            base,
            &mut self.classifications[source_range.as_usize()],
        );
        self.classify_command_substitutions(node);
    }

    // Classify a node as if it were an argument.
    fn classify_as_argument(
        &mut self,
        node: &dyn ast::Node,
        options_allowed: bool, /* = true */
    ) {
        // node does not necessarily have type symbol_argument here.
        let source_range = node.source_range();
        let arg_str = self.source_slice(source_range);
        let parameter = self.classification_of(TokenScopeId::PARAM);

        match option_shape(arg_str, options_allowed) {
            OptionShape::Argument => {
                classify_word(
                    arg_str,
                    self.enclosing_function_parameters.last(),
                    parameter,
                    &mut self.classifications[source_range.as_usize()],
                );
            }
            OptionShape::Whole(scope) => {
                let base = self.classification_of(scope);
                classify_word(
                    arg_str,
                    self.enclosing_function_parameters.last(),
                    base,
                    &mut self.classifications[source_range.as_usize()],
                );
            }
            OptionShape::LongWithValue { name_end } => {
                // The long form splits at its `=`: the name keeps the shape's scope and
                // the value is an argument in its own right.
                let name = self.classification_of(TokenScopeId::OPTION_LONG);
                let assignment = self.classification_of(TokenScopeId::OPERATOR_ASSIGNMENT);
                let parameters = self.enclosing_function_parameters.last();

                let classifications = &mut self.classifications[source_range.as_usize()];
                let (name_part, value_part) = classifications.split_at_mut(name_end + 1);
                classify_word(&arg_str[..name_end], parameters, name, name_part);
                name_part[name_end] = assignment;
                classify_word(&arg_str[name_end + 1..], parameters, parameter, value_part);
            }
        }

        self.classify_command_substitutions(node);
    }

    fn classify_command_substitutions(&mut self, node: &dyn ast::Node) {
        let source_range = node.source_range();
        let arg_str = self.source_slice(source_range);
        let arg_start = source_range.start();
        let mut cmdsub_cursor = 0;
        let mut is_quoted = false;
        while let Ok(Some(cmdsub)) = locate_cmdsubst_range(
            arg_str,
            &mut cmdsub_cursor,
            /*accept_incomplete=*/ true,
            Some(&mut is_quoted),
            None,
        ) {
            // Classify the parens. The open parens must exist; the closed paren may not if it was
            // incomplete.
            assert!(cmdsub.opening_paren_offset() < arg_str.len());
            let parenthesis = self.classification_of(TokenScopeId::OPERATOR_PARENTHESIS);
            self.classifications[arg_start..][cmdsub.opening_paren_range()].fill(parenthesis);
            self.classifications[arg_start..][cmdsub.closing_paren_range()].fill(parenthesis);

            // Classify it recursively.
            let arg_cursor = self
                .cursor
                .map(|c| c.wrapping_sub(arg_start + cmdsub.opening_paren_offset()));
            let cmdsub_contents = &arg_str[cmdsub.command_range()];
            let mut cmdsub_visitor = HighlightVisitor::inside(
                cmdsub_contents,
                arg_cursor,
                self.file_tester.ctx,
                self.working_directory,
                self.whether_io_permitted(),
                self.enclosing_scopes
                    .with_scope(TokenScopeId::META_COMMAND_SUBSTITUTION),
            );
            let sub_classifications = cmdsub_visitor.classify();

            // Copy out the sub-classifications back into our array.
            assert_eq!(sub_classifications.len(), cmdsub_contents.len());
            self.classifications[arg_start..][cmdsub.command_range()]
                .copy_from_slice(&sub_classifications);
        }
    }

    // Classifies the source range of a node with a given classification.
    fn classify_node(&mut self, node: &dyn ast::Node, classification: HighlightClassification) {
        self.classify_span(node.source_range(), classification);
    }

    // Classifies a range with a given classification.
    fn classify_span(&mut self, range: SourceRange, classification: HighlightClassification) {
        assert!(
            range.end() <= self.classifications.len(),
            "Range out of bounds"
        );
        self.classifications[range.start()..range.end()].fill(classification);
    }

    // Visit the children of a node.
    fn visit_children(&mut self, node: &dyn Node) {
        node.accept(self);
    }

    // AST visitor implementations.
    fn visit_keyword(&mut self, node: &dyn Keyword) {
        let scope = match node.keyword() {
            ParseKeyword::Begin => TokenScopeId::KEYWORD_BEGIN,
            ParseKeyword::Builtin => TokenScopeId::KEYWORD_BUILTIN,
            ParseKeyword::Case => TokenScopeId::KEYWORD_CASE,
            ParseKeyword::Command => TokenScopeId::KEYWORD_COMMAND,
            ParseKeyword::Else => TokenScopeId::KEYWORD_ELSE,
            ParseKeyword::End => TokenScopeId::KEYWORD_END,
            ParseKeyword::Exec => TokenScopeId::KEYWORD_EXEC,
            ParseKeyword::For => TokenScopeId::KEYWORD_FOR,
            ParseKeyword::Function => TokenScopeId::KEYWORD_FUNCTION,
            ParseKeyword::If => TokenScopeId::KEYWORD_IF,
            ParseKeyword::In => TokenScopeId::KEYWORD_IN,
            ParseKeyword::Switch => TokenScopeId::KEYWORD_SWITCH,
            ParseKeyword::While => TokenScopeId::KEYWORD_WHILE,
            ParseKeyword::And => TokenScopeId::KEYWORD_AND,
            ParseKeyword::Or => TokenScopeId::KEYWORD_OR,
            ParseKeyword::Not => TokenScopeId::KEYWORD_NOT,
            ParseKeyword::Exclam => TokenScopeId::KEYWORD_EXCLAM,
            ParseKeyword::Time => TokenScopeId::KEYWORD_TIME,
            ParseKeyword::None => TokenScopeId::NORMAL,
        };
        self.classify_node(node.as_node(), self.classification_of(scope));
    }

    fn visit_token(&mut self, tok: &dyn Token) {
        let scope = match tok.token_type() {
            ParseTokenType::Pipe => TokenScopeId::END_PIPE,
            ParseTokenType::Background => TokenScopeId::END_BACKGROUND,
            // An end token is a terminator, and its characters decide which one.
            ParseTokenType::End => TokenScopeId::END,
            ParseTokenType::LeftBrace | ParseTokenType::RightBrace => TokenScopeId::KEYWORD,
            ParseTokenType::AndAnd | ParseTokenType::OrOr => TokenScopeId::OPERATOR_LOGICAL,
            ParseTokenType::String => {
                // Assume all strings are params. This handles e.g. the variables a for header or
                // function header. Other strings (like arguments to commands) need more complex
                // handling, which occurs in their respective overrides of visit().
                TokenScopeId::PARAM
            }
            _ => TokenScopeId::NORMAL,
        };
        self.classify_node(tok.as_node(), self.classification_of(scope));
    }

    // Visit an argument, perhaps knowing that our command is cd.
    fn visit_argument(&mut self, arg: &Argument, cmd_is_cd: bool, options_allowed: bool) {
        self.classify_as_argument(arg, options_allowed);
        if !self.whether_io_permitted() {
            return;
        }

        // Mark every valid path.
        let source_range = arg.source_range();
        let is_prefix = self
            .cursor
            .is_some_and(|c| source_range.contains_inclusive(c));
        let token = arg.source(self.buff).to_owned();
        let test_result = if cmd_is_cd {
            if is_cd_option(&token) {
                Ok(IsFile(false))
            } else {
                self.file_tester.test_cd_path(&token, is_prefix)
            }
        } else {
            let is_path = self.file_tester.test_path(&token, is_prefix);
            Ok(IsFile(is_path))
        };
        match test_result {
            Ok(IsFile(false)) => (),
            Ok(IsFile(true)) => {
                for i in source_range.as_usize() {
                    self.classifications[i] =
                        self.classifications[i].with_attribute(HighlightAttribute::PathExisting);
                }
            }
            Err(..) => {
                let invalid = self.classification_of(TokenScopeId::ERROR_PATH);
                self.classify_node(arg, invalid);
            }
        }
    }

    fn visit_redirection(&mut self, redir: &Redirection) {
        // like 2>
        let oper = PipeOrRedir::try_from(redir.oper.source(self.buff))
            .expect("Should have successfully parsed a pipe_or_redir_t since it was in our ast");
        let target = redir.target.source(self.buff).to_owned(); // like &1 or file path

        // Classify the > part.
        // It may have parsed successfully yet still be invalid (e.g. 9999999999999>&1)
        // If so, classify the whole thing invalid and stop.
        if !oper.is_valid() {
            let invalid = self.classification_of(TokenScopeId::ERROR_REDIRECTION);
            self.classify_node(redir, invalid);
            return;
        }

        // The mode the operator spells classifies both the operator and its target.
        let redirection = self.classification_of(redirection_mode_scope(oper.mode));
        self.classify_node(&redir.oper, redirection);

        // Classify the target part.
        // Check if the argument contains a command substitution. If so, classify it as a param
        // even though it's a command redirection, and don't try to do any other validation.
        if has_cmdsub(&target) {
            self.classify_as_argument(&redir.target, true);
            return;
        }
        // No command substitution, so we can classify the target file or fd. For example,
        // disallow redirections into a non-existent directory.
        let (scope, file_exists) = if !self.whether_io_permitted() {
            // I/O is disallowed, so we don't have much hope of catching anything but gross
            // errors. Assume it's valid.
            (redirection_mode_scope(oper.mode), false)
        } else if contains_pending_variable(&self.pending_variable_names, &target) {
            // Target uses a variable defined by the current commandline. Assume it's valid.
            (redirection_mode_scope(oper.mode), false)
        } else {
            // Validate the redirection target..
            if let Ok(IsFile(file_exists)) =
                self.file_tester.test_redirection_target(&target, oper.mode)
            {
                (redirection_mode_scope(oper.mode), file_exists)
            } else {
                (TokenScopeId::ERROR_REDIRECTION, false)
            }
        };
        let classification = self.classification_of(scope);
        self.classify_node(&redir.target, classification);
        if file_exists {
            for i in redir.target.source_range().as_usize() {
                self.classifications[i] =
                    self.classifications[i].with_attribute(HighlightAttribute::PathExisting);
            }
        }
    }

    fn visit_variable_assignment(&mut self, varas: &VariableAssignment) {
        self.classify_as_argument(varas, true);
        // Classify the '=' in variable assignments as an assignment operator, and the name
        // before it as the definition it is.
        if let Some(offset) = variable_assignment_equals_pos(varas.source(self.buff)) {
            let start = varas.source_range().start();
            // An assignment in command position defines a local, exported variable.
            let definition = self.classification_of(definition_scope(&EnvMode::LOCAL_EXPORTED));
            self.classifications[start..start + offset].fill(definition);
            let assignment = self.classification_of(TokenScopeId::OPERATOR_ASSIGNMENT);
            self.classifications[start + offset] = assignment;
            let var_name = &varas.source(self.buff)[..offset];
            self.pending_variable_names.push(var_name);
        }
    }

    fn visit_semi_nl(&mut self, node: &dyn Node) {
        self.classify_terminator_span(node.source_range());
    }

    /// Classify a terminator's characters: a semicolon and a newline part company.
    fn classify_terminator_span(&mut self, range: SourceRange) {
        let semicolon = self.classification_of(TokenScopeId::END_SEMICOLON);
        let newline = self.classification_of(TokenScopeId::END_NEWLINE);
        let text = self.source_slice(range);
        let classifications = &mut self.classifications[range.as_usize()];
        for (index, character) in text.chars().enumerate() {
            classifications[index] = if character == ';' { semicolon } else { newline };
        }
    }

    fn visit_decorated_statement(&mut self, stmt: &DecoratedStatement) {
        // Classify any decoration.
        if let Some(decoration) = stmt.opt_decoration.as_ref() {
            self.visit_keyword(&decoration.keyword);
            if let Some(separator) = &decoration.separator {
                let option = self.classification_of(TokenScopeId::OPTION);
                self.classify_node(separator, option);
            }
        }

        // Classify the command's source code.
        // If we get no source back, there's nothing to classify.
        if stmt.command.try_source_range().is_none() {
            return;
        }
        let cmd = stmt.command.source(self.buff);

        // A word the shell reserves names one thing in every pass: no function can take the
        // name, so the text decides the scope and neither resolution nor I/O is involved.
        // A word the shell does not reserve is resolved like any other command word, so its
        // scope names the builtin only where the resolution found one.
        // A command or exec decoration looks the word up as a program instead, which a
        // reserved word cannot be.
        let reserved_word_scope = match stmt.decoration() {
            StatementDecoration::None | StatementDecoration::Builtin
                if parser_keywords_is_reserved(cmd) =>
            {
                keyword_word_scope(cmd).or_else(|| builtin_word_scope(cmd))
            }
            _ => None,
        };

        let mut expanded_cmd = WString::new();
        let mut cmd_scope = None;
        let mut unresolved_command = false;
        if reserved_word_scope.is_none() && !has_cmdsub_without_dollar(cmd) {
            // We cannot check a command substitution without executing it, so assume it is valid.
            if !self.whether_io_permitted()
                || variable_assignment_equals_pos(cmd).is_some()
                || has_cmdsub(cmd)
            {
                // A command whose kind this pass cannot determine: the family node is the
                // only scope that claims no kind.
                cmd_scope = Some(TokenScopeId::COMMAND);
            } else {
                // Check to see if the command is valid.
                // Try expanding it. If we cannot, the statement is a syntax error.
                if let Some(expanded) =
                    statement_get_expanded_command(self.buff, stmt, self.file_tester.ctx)
                {
                    expanded_cmd = expanded;
                    if !has_expand_reserved(&expanded_cmd) {
                        cmd_scope = command_scope(
                            &expanded_cmd,
                            stmt.decoration(),
                            self.working_directory,
                            self.file_tester.ctx.vars(),
                        );
                        // The word expanded, so no command-position test matching it is a
                        // fact about the word rather than about the parse.
                        unresolved_command = cmd_scope.is_none();
                    }
                }
            }
        }

        // Classify our statement.
        if let Some(cmd_scope) = reserved_word_scope.or(cmd_scope) {
            self.classify_command_word(&stmt.command, cmd_scope);
        } else {
            let scope = if unresolved_command {
                TokenScopeId::ERROR_COMMAND
            } else {
                TokenScopeId::ERROR
            };
            let invalid = self.classification_of(scope);
            self.classify_node(&stmt.command, invalid);
        }

        // Classify arguments and redirections.
        // Except if our command is 'cd' we have special logic for how arguments are classified.
        let is_cd = is_veritable_cd(&expanded_cmd);
        // A definition command's names take the scope of the mode its flags request.
        // The word is read from the buffer, so a pass that resolves no commands — no I/O, a
        // pager row, `fish_indent` — still classifies the definitions the text declares; a
        // decorated word is some other command, not the builtin that defines.
        let definitions = (stmt.decoration() == StatementDecoration::None)
            .then(|| DefinitionCommand::of_command(stmt.command.source(self.buff)))
            .flatten()
            .and_then(|command| {
                definition_mode(stmt.args_or_redirs.iter(), self.buff).map(|mode| (command, mode))
            });
        let mut defining = definitions.is_some();
        // If we have seen a "--" argument, classify all options from then on as normal arguments.
        let mut have_dashdash = false;
        for v in &stmt.args_or_redirs {
            if v.is_argument() {
                let name = v.argument().source(self.buff);
                let mut definition = None;
                if defining
                    && let Some((command, mode)) = definitions
                    && valid_var_name(name)
                {
                    definition = Some(definition_scope(&mode));
                    self.pending_variable_names.push(name);
                    defining = command.whether_defines_each_name();
                }
                self.visit_argument(v.argument(), is_cd, !have_dashdash);
                if let Some(scope) = definition {
                    let classification = self.classification_of(scope);
                    self.classify_node(v.argument(), classification);
                }
                if name == "--" {
                    have_dashdash = true;
                }
            } else {
                self.visit_redirection(v.redirection());
            }
        }
    }

    fn visit_block_statement(&mut self, block: &BlockStatement) {
        match &block.header {
            BlockStatementHeader::For(node) => self.visit(node),
            BlockStatementHeader::While(node) => self.visit(node),
            BlockStatementHeader::Function(node) => self.visit(node),
            BlockStatementHeader::Begin(node) => self.visit(node),
        }
        self.visit(&block.args_or_redirs);
        let pending_variable_names_count = self.pending_variable_names.len();

        // A definition's body resolves references against the parameters its own header
        // declares.
        let parameters = match &block.header {
            BlockStatementHeader::Function(header) => {
                Some(FunctionParameterList::of_function_header(header, self.buff))
            }
            _ => None,
        };
        let has_parameters = parameters.is_some();
        self.enclosing_function_parameters.extend(parameters);

        if let BlockStatementHeader::For(fh) = &block.header {
            let var_name = fh.var_name.source(self.buff);
            self.pending_variable_names.push(var_name);

            // A loop's variable is defined by the header, in the local scope.
            let scope = definition_scope(&EnvMode::LOCAL);
            let definition = self.classification_of(scope);
            self.classify_node(&fh.var_name, definition);
        }

        // The names a definition's header declares are defined in its local scope.
        if let BlockStatementHeader::Function(header) = &block.header {
            let name_indices = self
                .enclosing_function_parameters
                .last()
                .map(|parameters| parameters.argument_name_indices().to_vec())
                .unwrap_or_default();
            let scope = definition_scope(&EnvMode::LOCAL);
            for index in name_indices {
                let classification = self.classification_of(scope);
                match index {
                    0 => self.classify_node(&header.first_arg, classification),
                    _ => self.classify_node(&header.args[index - 1], classification),
                }
            }
        }
        let body_scopes = body_enclosing(&block.header, self.enclosing_function_parameters.last());
        self.visit_inside(body_scopes, |visitor| visitor.visit(&block.jobs));
        self.visit(&block.end);
        if has_parameters {
            self.enclosing_function_parameters.pop();
        }
        self.pending_variable_names
            .truncate(pending_variable_names_count);
    }

    fn visit_brace_statement(&mut self, brace_statement: &BraceStatement) {
        self.visit(&brace_statement.left_brace);
        self.visit(&brace_statement.args_or_redirs);
        self.visit_inside(enclosing_of(TokenScopeId::META_BLOCK), |visitor| {
            visitor.visit(&brace_statement.jobs);
        });
        self.visit(&brace_statement.right_brace);
    }
}

/// Return whether a string contains a command substitution.
fn has_cmdsub(src: &wstr) -> bool {
    let mut cursor = 0;
    match locate_cmdsubst_range(src, &mut cursor, true, None, None) {
        Err(()) | Ok(None) => false,
        Ok(Some(_)) => true,
    }
}

fn has_cmdsub_without_dollar(src: &wstr) -> bool {
    let mut cursor = 0;
    let mut is_quoted = false;
    loop {
        let mut has_dollar = false;
        match locate_cmdsubst_range(
            src,
            &mut cursor,
            true,
            Some(&mut is_quoted),
            Some(&mut has_dollar),
        ) {
            Err(()) | Ok(None) => return false,
            Ok(Some(_)) if !has_dollar => return true,
            Ok(Some(_)) => {}
        }
    }
}

fn contains_pending_variable(pending_variable_names: &[&wstr], haystack: &wstr) -> bool {
    for var_name in pending_variable_names {
        let mut nextpos = 0;
        while let Some(relpos) = &haystack[nextpos..]
            .as_char_slice()
            .windows(var_name.len())
            .position(|w| w == var_name.as_char_slice())
        {
            let pos = nextpos + relpos;
            nextpos = pos + 1;
            if pos == 0 || haystack.as_char_slice()[pos - 1] != '$' {
                continue;
            }
            let end = pos + var_name.len();
            if end < haystack.len() && valid_var_name_char(haystack.as_char_slice()[end]) {
                continue;
            }
            return true;
        }
    }
    false
}

impl<'src, 'wd, 'ctx, 'a> NodeVisitor<'a> for HighlightVisitor<'src, 'wd, 'ctx> {
    fn visit(&mut self, node: &'a dyn Node) {
        if let Some(keyword) = node.as_keyword() {
            return self.visit_keyword(keyword);
        }
        if let Some(token) = node.as_token() {
            if token.token_type() == ParseTokenType::End {
                self.visit_semi_nl(node);
                return;
            }
            self.visit_token(token);
            return;
        }
        match node.kind() {
            Kind::Argument(node) => self.visit_argument(node, false, true),
            Kind::Redirection(node) => self.visit_redirection(node),
            Kind::VariableAssignment(node) => self.visit_variable_assignment(node),
            Kind::DecoratedStatement(node) => self.visit_decorated_statement(node),
            Kind::TimeDecorator(node) => {
                self.visit_keyword(&node.keyword);
                if let Some(separator) = &node.separator {
                    let option = self.classification_of(TokenScopeId::OPTION);
                    self.classify_node(separator, option);
                }
            }
            Kind::BlockStatement(node) => self.visit_block_statement(node),
            Kind::BraceStatement(node) => self.visit_brace_statement(node),
            Kind::IfClause(node) => {
                self.visit(&node.kw_if);
                self.visit(&node.condition);
                self.visit(&node.andor_tail);
                self.visit_inside(enclosing_of(TokenScopeId::META_IF), |visitor| {
                    visitor.visit(&node.body);
                });
            }
            Kind::ElseClause(node) => {
                self.visit(&node.kw_else);
                if let Some(semi_nl) = &node.semi_nl {
                    self.visit(semi_nl);
                }
                self.visit_inside(enclosing_of(TokenScopeId::META_IF), |visitor| {
                    visitor.visit(&node.body);
                });
            }
            Kind::CaseItem(node) => {
                self.visit(&node.kw_case);
                self.visit(&node.arguments);
                self.visit(&node.semi_nl);
                self.visit_inside(enclosing_of(TokenScopeId::META_CASE), |visitor| {
                    visitor.visit(&node.body);
                });
            }
            Kind::JobPipeline(node) if !node.continuation.is_empty() => {
                self.visit_inside(enclosing_of(TokenScopeId::META_PIPELINE), |visitor| {
                    visitor.visit_children(node);
                });
            }
            Kind::JobConjunction(node)
                if node.decorator.is_some() || !node.continuations.is_empty() =>
            {
                self.visit_inside(enclosing_of(TokenScopeId::META_AND_OR), |visitor| {
                    visitor.visit_children(node);
                });
            }
            // Default implementation is to just visit children.
            _ => self.visit_children(node),
        }
    }
}

// Given a plain statement node in a parse tree, get the command and return it, expanded
// appropriately for commands. If we succeed, return true.
fn statement_get_expanded_command(
    src: &wstr,
    stmt: &ast::DecoratedStatement,
    ctx: &mut OperationContext<'_>,
) -> Option<WString> {
    // Get the command. Try expanding it. If we cannot, it's an error.
    let cmd = stmt.command.try_source(src)?;
    let mut out_cmd = WString::new();
    let err = expand_to_command_and_args(
        cmd,
        ctx,
        &mut out_cmd,
        None,
        None,
        /*skip_cmdsubs=*/ true,
        /*skip_wildcards=*/ false,
    );
    (err == ExpandResultCode::Ok).then_some(out_cmd)
}

#[cfg(test)]
mod tests {
    use super::highlight_shell;
    use crate::env::{EnvMode, EnvSetMode, Environment};
    use crate::highlight::HighlightClassification;
    use crate::operation_context::{EXPANSION_LIMIT_BACKGROUND, OperationContext};
    use crate::prelude::*;
    use crate::tests::prelude::*;
    use fish_common::ScopeGuard;
    use fish_feature_flags::{FeatureFlag, with_overridden_feature};
    use fish_token_scope::{EnclosingScopeSet, HighlightAttribute, TokenScopeId};
    use libc::PATH_MAX;

    // Helper to return a string whose length greatly exceeds PATH_MAX.
    fn get_overlong_path() -> String {
        let path_max = usize::try_from(PATH_MAX).unwrap();
        let mut longpath = String::with_capacity(path_max * 2 + 10);
        while longpath.len() <= path_max * 2 {
            longpath += "/overlong";
        }
        longpath
    }

    #[test]
    #[serial]
    fn test_highlighting() {
        test_init();
        let TestParser {
            ref mut parser,
            ref mut pushed_dirs,
        } = TestParser::new();
        // Testing syntax highlighting
        parser.pushd(pushed_dirs, "test/fish_highlight_test/");
        let parser = &mut **ScopeGuard::new(parser, |parser| parser.popd(pushed_dirs));
        std::fs::create_dir_all("dir").unwrap();
        std::fs::create_dir_all("cdpath-entry/dir-in-cdpath").unwrap();
        std::fs::write("foo", []).unwrap();
        std::fs::write("bar", []).unwrap();
        std::fs::write("-filename-starting-with-dash", []).unwrap();

        // Here are the components of our source and the classifications we expect those to be.
        #[derive(Debug)]
        struct HighlightComponent<'a> {
            text: &'a str,
            classification: HighlightClassification,
            nospace: bool,
        }

        macro_rules! component {
            ( ( $text:expr, $classification:expr) ) => {
                HighlightComponent {
                    text: $text,
                    classification: $classification,
                    nospace: false,
                }
            };
            ( ( $text:literal, $classification:expr, ns ) ) => {
                HighlightComponent {
                    text: $text,
                    classification: $classification,
                    nospace: true,
                }
            };
        }

        macro_rules! validate {
            ( $($comp:tt),* $(,)? ) => {
                let components = [
                    $(
                        component!($comp),
                    )*
                ];
                let vars = parser.vars();
                // Generate the text.
                let mut text = WString::new();
                let mut expected_classifications = vec![];
                for comp in &components {
                    if !text.is_empty() && !comp.nospace {
                        text.push(' ');
                        expected_classifications.push(HighlightClassification::default());
                    }
                    text.push_str(comp.text);
                    expected_classifications.resize(text.len(), comp.classification);
                }
                assert_eq!(text.len(), expected_classifications.len());

                let mut classifications = vec![];
                highlight_shell(
                    &text,
                    &mut classifications,
                    &mut OperationContext::background(vars, EXPANSION_LIMIT_BACKGROUND),
                    true, /* io_ok */
                    Some(text.len()),
                );
                assert_eq!(classifications.len(), expected_classifications.len());

                for (i, c) in text.chars().enumerate() {
                    // Hackish space handling. We don't care about the classifications in spaces.
                    if c == ' ' {
                        continue;
                    }

                    // The component states the scope, the style fields and the attributes it
                    // sees; the enclosing scopes a region adds are asserted by their own test.
                    assert_eq!(
                        (
                            classifications[i].foreground(),
                            classifications[i].background(),
                            classifications[i].attributes(),
                        ),
                        (
                            expected_classifications[i].foreground(),
                            expected_classifications[i].background(),
                            expected_classifications[i].attributes(),
                        ),
                        "Failed at position {i}, char {c}, of {text:?}"
                    );
                }
            };
        }

        let param_valid_path = HighlightClassification::from(TokenScopeId::PARAM)
            .with_attribute(HighlightAttribute::PathExisting);

        let redirection_valid_path = |scope: TokenScopeId| {
            HighlightClassification::from(scope).with_attribute(HighlightAttribute::PathExisting)
        };

        with_overridden_feature(FeatureFlag::AmpersandNoBgInToken, true, || {
            let fg = HighlightClassification::from;

            // Verify variables and wildcards in commands using /bin/cat.
            let vars = parser.vars();
            let local_mode = EnvSetMode::new_at_early_startup(EnvMode::LOCAL);
            vars.set_one(L!("CDPATH"), local_mode, L!("./cdpath-entry").to_owned());

            // NOTE n, nv are suffix of /usr/bin/env
            vars.set_one(L!("VARIABLE_IN_COMMAND"), local_mode, L!("n").to_owned());
            vars.set_one(L!("VARIABLE_IN_COMMAND2"), local_mode, L!("nv").to_owned());

            let _cleanup = ScopeGuard::new((), |_| {
                vars.remove(L!("VARIABLE_IN_COMMAND"), EnvSetMode::default());
                vars.remove(L!("VARIABLE_IN_COMMAND2"), EnvSetMode::default());
            });

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("./foo", param_valid_path),
                ("&", fg(TokenScopeId::END_BACKGROUND)),
            );

            // A builtin the declaration names has a scope of its own; one it does not name
            // keeps the builtin kind.
            validate!(
                ("set", fg(TokenScopeId::BUILTIN_SET)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("x", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
            );
            validate!(
                ("eval", fg(TokenScopeId::BUILTIN_EVAL)),
                ("echo", fg(TokenScopeId::PARAM)),
            );
            validate!(
                ("read", fg(TokenScopeId::BUILTIN_READ)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("x", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
            );
            validate!(
                ("test", fg(TokenScopeId::BUILTIN_TEST)),
                ("-e", fg(TokenScopeId::OPTION_SHORT)),
                ("/tmp", param_valid_path),
            );
            validate!(
                ("status", fg(TokenScopeId::BUILTIN_STATUS)),
                ("is-interactive", fg(TokenScopeId::PARAM)),
            );
            validate!(
                ("random", fg(TokenScopeId::BUILTIN_RANDOM)),
                ("1", fg(TokenScopeId::PARAM)),
            );
            validate!(
                ("emit", fg(TokenScopeId::BUILTIN_ASYNC_EMIT)),
                ("evt", fg(TokenScopeId::PARAM)),
            );
            // The shipped `wait` function is what the word runs, so the builtin's own scope
            // is reached by asking for the builtin.
            validate!(("wait", fg(TokenScopeId::FUNCTION)),);
            validate!(
                ("builtin", fg(TokenScopeId::KEYWORD_BUILTIN)),
                ("wait", fg(TokenScopeId::BUILTIN_ASYNC_WAIT)),
            );
            validate!(("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),);
            validate!(("false", fg(TokenScopeId::BUILTIN_BOOLEAN_FALSE)),);
            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("/tmp", param_valid_path),
            );

            // A reserved word cannot direct control flow and a command at once: the keyword's
            // text wins its own scope.
            validate!(
                ("return", fg(TokenScopeId::KEYWORD_RETURN)),
                ("2", fg(TokenScopeId::PARAM)),
            );
            validate!(("break", fg(TokenScopeId::KEYWORD_BREAK)),);
            validate!(("continue", fg(TokenScopeId::KEYWORD_CONTINUE)),);

            validate!(
                ("command", fg(TokenScopeId::KEYWORD_COMMAND)),
                ("echo", fg(TokenScopeId::COMMAND_EXTERNAL)),
                ("abc", fg(TokenScopeId::PARAM)),
                ("foo", param_valid_path),
                ("&", fg(TokenScopeId::END_BACKGROUND)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("foo&bar", fg(TokenScopeId::PARAM)),
                ("foo", fg(TokenScopeId::PARAM), ns),
                ("&", fg(TokenScopeId::END_BACKGROUND)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("&>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
            );

            validate!(
                ("if", fg(TokenScopeId::KEYWORD_IF)),
                ("command", fg(TokenScopeId::KEYWORD_COMMAND)),
                ("ls", fg(TokenScopeId::COMMAND_EXTERNAL)),
                ("; ", fg(TokenScopeId::END_SEMICOLON)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("abc", fg(TokenScopeId::PARAM)),
                ("; ", fg(TokenScopeId::END_SEMICOLON)),
                (
                    "/bin/definitely_not_a_command",
                    fg(TokenScopeId::ERROR_COMMAND)
                ),
                ("; ", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            validate!(
                ("if", fg(TokenScopeId::KEYWORD_IF)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("else", fg(TokenScopeId::KEYWORD_ELSE)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            // Verify that cd shows errors for non-directories.
            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("dir", param_valid_path),
            );

            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("foo", fg(TokenScopeId::ERROR_PATH)),
            );

            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("--help", fg(TokenScopeId::OPTION_LONG)),
                ("-h", fg(TokenScopeId::OPTION_SHORT)),
                ("definitely_not_a_directory", fg(TokenScopeId::ERROR_PATH)),
            );

            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("--logical", fg(TokenScopeId::OPTION_LONG)),
                (
                    "still_definitely_not_a_directory",
                    fg(TokenScopeId::ERROR_PATH)
                ),
            );

            validate!(
                ("cd", fg(TokenScopeId::FUNCTION)),
                ("dir-in-cdpath", param_valid_path),
            );

            // Command substitutions.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("param1", fg(TokenScopeId::PARAM)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("--", fg(TokenScopeId::OPTION)),
                ("-l", fg(TokenScopeId::PARAM)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
                ("ls", fg(TokenScopeId::FUNCTION)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("--", fg(TokenScopeId::OPTION)),
                ("-l", fg(TokenScopeId::PARAM)),
                ("param2", fg(TokenScopeId::PARAM)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
                ("|", fg(TokenScopeId::END_PIPE)),
                ("cat", fg(TokenScopeId::COMMAND_EXTERNAL)),
            );
            validate!(
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("$", fg(TokenScopeId::OPERATOR_DOLLAR)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS), ns),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
            );
            validate!(
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER)),
                ("before", fg(TokenScopeId::QUOTE_DOUBLE), ns),
                ("$", fg(TokenScopeId::OPERATOR_DOLLAR)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS), ns),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("param1", fg(TokenScopeId::PARAM)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
                ("after", fg(TokenScopeId::QUOTE_DOUBLE), ns),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER), ns),
                ("param2", fg(TokenScopeId::PARAM)),
            );
            validate!(
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("\"", fg(TokenScopeId::ERROR)),
                ("unclosed quote", fg(TokenScopeId::QUOTE_DOUBLE), ns),
                ("$", fg(TokenScopeId::OPERATOR_DOLLAR)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS), ns),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
            );

            // Redirections substitutions.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("param1", fg(TokenScopeId::PARAM)),
                // Input redirection.
                ("<", fg(TokenScopeId::REDIRECTION_INPUT)),
                (
                    "/dev/null",
                    redirection_valid_path(TokenScopeId::REDIRECTION_INPUT)
                ),
                // Output redirection to a valid fd.
                ("1>&2", fg(TokenScopeId::REDIRECTION_FD)),
                // Output redirection to an invalid fd.
                ("2>&", fg(TokenScopeId::REDIRECTION_FD)),
                ("LO", fg(TokenScopeId::ERROR_REDIRECTION)),
                // Just a param, not a redirection.
                ("test/blah", fg(TokenScopeId::PARAM)),
                // Input redirection from directory.
                ("<", fg(TokenScopeId::REDIRECTION_INPUT)),
                ("test/", fg(TokenScopeId::ERROR_REDIRECTION)),
                // Output redirection to an invalid path.
                ("3>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                (
                    "/not/a/valid/path/nope",
                    fg(TokenScopeId::ERROR_REDIRECTION)
                ),
                // Output redirection to directory.
                ("3>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("test/nope/", fg(TokenScopeId::ERROR_REDIRECTION)),
                // Redirections to overflow fd.
                (
                    "99999999999999999999>&2",
                    fg(TokenScopeId::ERROR_REDIRECTION)
                ),
                ("2>&", fg(TokenScopeId::REDIRECTION_FD)),
                ("99999999999999999999", fg(TokenScopeId::ERROR_REDIRECTION)),
                // Output redirection containing a command substitution.
                ("4>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("test/somewhere", fg(TokenScopeId::PARAM)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
                // Just another param.
                ("param2", fg(TokenScopeId::PARAM)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                (">", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("-no-such-file", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                (">", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                (
                    "-filename-starting-with-dash",
                    redirection_valid_path(TokenScopeId::REDIRECTION_OUTPUT)
                ),
            );

            validate!(
                ("$", fg(TokenScopeId::OPERATOR_DOLLAR)),
                ("(", fg(TokenScopeId::OPERATOR_PARENTHESIS), ns),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (")", fg(TokenScopeId::OPERATOR_PARENTHESIS)),
            );
            validate!(
                ("(", fg(TokenScopeId::ERROR)),
                ("true", fg(TokenScopeId::ERROR)),
                (")", fg(TokenScopeId::ERROR)),
            );
            validate!(("$(echo echo)(echo invalid)", fg(TokenScopeId::ERROR)));

            validate!(
                ("for", fg(TokenScopeId::KEYWORD_FOR)),
                ("x", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
                ("in", fg(TokenScopeId::KEYWORD_IN)),
                ("set-by-for-1", fg(TokenScopeId::PARAM)),
                ("set-by-for-2", fg(TokenScopeId::PARAM)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                (">", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("$x", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            validate!(
                ("set", fg(TokenScopeId::BUILTIN_SET)),
                ("x", fg(TokenScopeId::VARIABLE_DEFINITION)),
                ("set-by-set", fg(TokenScopeId::PARAM)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                (">", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("$x", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("2>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("$totally_not_x", fg(TokenScopeId::ERROR_REDIRECTION)),
                ("<", fg(TokenScopeId::REDIRECTION_INPUT)),
                (
                    "$x_but_its_an_impostor",
                    fg(TokenScopeId::ERROR_REDIRECTION)
                ),
            );

            validate!(
                (
                    "x",
                    fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED),
                    ns
                ),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
                ("set-by-variable-override", fg(TokenScopeId::PARAM), ns),
                ("echo", fg(TokenScopeId::BUILTIN)),
                (">", fg(TokenScopeId::REDIRECTION_OUTPUT)),
                ("$x", fg(TokenScopeId::REDIRECTION_OUTPUT)),
            );

            validate!(
                ("end", fg(TokenScopeId::ERROR)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("if", fg(TokenScopeId::KEYWORD_IF)),
                ("end", fg(TokenScopeId::ERROR)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("'", fg(TokenScopeId::ERROR)),
                ("single_quote", fg(TokenScopeId::QUOTE_SINGLE), ns),
                ("$stuff", fg(TokenScopeId::QUOTE_SINGLE), ns),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("\"", fg(TokenScopeId::ERROR)),
                ("double_quote", fg(TokenScopeId::QUOTE_DOUBLE), ns),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL), ns),
                ("stuff", fg(TokenScopeId::VARIABLE), ns),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL)),
                ("foo", fg(TokenScopeId::VARIABLE), ns),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER)),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL), ns),
                ("bar", fg(TokenScopeId::VARIABLE), ns),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER), ns),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL)),
                ("baz", fg(TokenScopeId::VARIABLE), ns),
                ("[", fg(TokenScopeId::VARIABLE_SLICE), ns),
                ("1 2..3", fg(TokenScopeId::PARAM)),
                ("]", fg(TokenScopeId::VARIABLE_SLICE)),
            );

            // An option's shape is a spelling, not a reading.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("-s", fg(TokenScopeId::OPTION_SHORT)),
                ("--long", fg(TokenScopeId::OPTION_LONG)),
                ("-gnu", fg(TokenScopeId::OPTION_GNU)),
                ("-abc", fg(TokenScopeId::OPTION_GNU)),
                ("-5", fg(TokenScopeId::OPTION_NUMERIC)),
                ("-", fg(TokenScopeId::OPTION)),
                ("--", fg(TokenScopeId::OPTION)),
                ("-s", fg(TokenScopeId::PARAM)),
            );

            // The long form splits at its `=`.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("--colour", fg(TokenScopeId::OPTION_LONG)),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
                ("always", fg(TokenScopeId::PARAM), ns),
            );

            // A mixed word: the quoted runs take their quote style, the rest stays the
            // argument.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("a", fg(TokenScopeId::PARAM)),
                ("'", fg(TokenScopeId::QUOTE_SINGLE_DELIMITER), ns),
                ("b", fg(TokenScopeId::QUOTE_SINGLE), ns),
                ("'", fg(TokenScopeId::QUOTE_SINGLE_DELIMITER), ns),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER), ns),
                ("c", fg(TokenScopeId::QUOTE_DOUBLE), ns),
                ("\"", fg(TokenScopeId::QUOTE_DOUBLE_DELIMITER), ns),
            );

            // An escape is classified by the shape of what follows the backslash.
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("\\u1234", fg(TokenScopeId::ESCAPE_UNICODE)),
                ("\\123", fg(TokenScopeId::ESCAPE_OCTAL)),
                ("\\c", fg(TokenScopeId::ESCAPE_CONTROL)),
                ("i", fg(TokenScopeId::PARAM), ns),
                ("\\n", fg(TokenScopeId::ESCAPE)),
            );

            validate!(
                ("for", fg(TokenScopeId::KEYWORD_FOR)),
                ("i", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
                ("in", fg(TokenScopeId::KEYWORD_IN)),
                ("1 2 3", fg(TokenScopeId::PARAM)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL_DEREFERENCE)),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL), ns),
                ("foo", fg(TokenScopeId::VARIABLE), ns),
                ("[", fg(TokenScopeId::VARIABLE_SLICE), ns),
                ("1", fg(TokenScopeId::PARAM)),
                ("]", fg(TokenScopeId::VARIABLE_SLICE), ns),
                ("[", fg(TokenScopeId::VARIABLE_SLICE), ns),
                ("2", fg(TokenScopeId::PARAM)),
                ("]", fg(TokenScopeId::VARIABLE_SLICE)),
                ("[3]", fg(TokenScopeId::PARAM)), // two dollar signs, so last one is not an expansion
            );

            validate!(
                ("cat", fg(TokenScopeId::COMMAND_EXTERNAL)),
                ("/dev/null", param_valid_path),
                ("|", fg(TokenScopeId::END_PIPE)),
                // This is bogus, but we used to use "less" here and that doesn't have to be installed.
                ("cat", fg(TokenScopeId::COMMAND_EXTERNAL)),
                ("2>", fg(TokenScopeId::REDIRECTION_OUTPUT)),
            );

            // Highlight path-prefixes only at the cursor.
            validate!(
                ("cat", fg(TokenScopeId::COMMAND_EXTERNAL)),
                ("/dev/nu", fg(TokenScopeId::PARAM)),
                ("/dev/nu", param_valid_path),
            );

            validate!(
                ("if", fg(TokenScopeId::KEYWORD_IF)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("&&", fg(TokenScopeId::OPERATOR_LOGICAL)),
                ("false", fg(TokenScopeId::BUILTIN_BOOLEAN_FALSE)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("or", fg(TokenScopeId::KEYWORD_OR)),
                ("false", fg(TokenScopeId::BUILTIN_BOOLEAN_FALSE)),
                ("||", fg(TokenScopeId::OPERATOR_LOGICAL)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("and", fg(TokenScopeId::KEYWORD_AND)),
                ("not", fg(TokenScopeId::KEYWORD_NOT)),
                ("!", fg(TokenScopeId::KEYWORD_EXCLAM)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("%self", fg(TokenScopeId::OPERATOR)),
                ("not%self", fg(TokenScopeId::PARAM)),
                ("self%not", fg(TokenScopeId::PARAM)),
            );

            validate!(
                ("false", fg(TokenScopeId::BUILTIN_BOOLEAN_FALSE)),
                ("&|", fg(TokenScopeId::END_PIPE)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
            );

            validate!(
                ("HOME", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED)),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
                (".", fg(TokenScopeId::PARAM), ns),
                ("VAR1", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED)),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
                ("VAL1", fg(TokenScopeId::PARAM), ns),
                ("VAR", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED)),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
                ("false", fg(TokenScopeId::BUILTIN_BOOLEAN_FALSE)),
                ("|&", fg(TokenScopeId::END_PIPE)),
                ("true", fg(TokenScopeId::BUILTIN_BOOLEAN_TRUE)),
                ("stuff", fg(TokenScopeId::PARAM)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)), // (
                (")", fg(TokenScopeId::ERROR)),
            );

            // A definition command's names take the mode its flags request.
            validate!(
                ("set", fg(TokenScopeId::BUILTIN_SET)),
                ("-gx", fg(TokenScopeId::OPTION_GNU)),
                (
                    "name",
                    fg(TokenScopeId::VARIABLE_DEFINITION_GLOBAL_EXPORTED)
                ),
                ("value", fg(TokenScopeId::PARAM)),
            );
            validate!(
                ("read", fg(TokenScopeId::BUILTIN_READ)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("name", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
            );
            validate!(
                ("set", fg(TokenScopeId::BUILTIN_SET)),
                ("-l", fg(TokenScopeId::OPTION_SHORT)),
                ("--", fg(TokenScopeId::OPTION)),
                ("name", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
                ("value", fg(TokenScopeId::PARAM)),
            );
            // An option the classifier cannot read makes it claim no definition.
            validate!(
                ("set", fg(TokenScopeId::BUILTIN_SET)),
                ("-e", fg(TokenScopeId::OPTION_SHORT)),
                ("name", fg(TokenScopeId::PARAM)),
            );

            // A name a definition's header declares is defined in its local scope.
            validate!(
                ("function", fg(TokenScopeId::KEYWORD_FUNCTION)),
                ("f", fg(TokenScopeId::PARAM)),
                ("-a", fg(TokenScopeId::OPTION_SHORT)),
                ("name", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL)),
                (";", fg(TokenScopeId::END_SEMICOLON)),
                ("end", fg(TokenScopeId::KEYWORD_END)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("stuff", fg(TokenScopeId::PARAM)),
                ("# comment", fg(TokenScopeId::COMMENT)),
            );

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("--", fg(TokenScopeId::OPTION)),
                ("-s", fg(TokenScopeId::PARAM)),
            );

            for (decorator, scope) in [
                ("command", TokenScopeId::KEYWORD_COMMAND),
                ("exec", TokenScopeId::KEYWORD_EXEC),
            ] {
                validate!(
                    (decorator, fg(scope)),
                    ("--", fg(TokenScopeId::OPTION)),
                    ("/bin/cat", fg(TokenScopeId::COMMAND_EXTERNAL)),
                    ("-n", fg(TokenScopeId::OPTION_SHORT)),
                    ("--", fg(TokenScopeId::OPTION)),
                    ("-s", fg(TokenScopeId::PARAM)),
                );
            }

            for (decorator, scope) in [
                ("builtin", TokenScopeId::KEYWORD_BUILTIN),
                ("time", TokenScopeId::KEYWORD_TIME),
            ] {
                validate!(
                    (decorator, fg(scope)),
                    ("--", fg(TokenScopeId::OPTION)),
                    ("echo", fg(TokenScopeId::BUILTIN)),
                    ("-n", fg(TokenScopeId::OPTION_SHORT)),
                );
            }

            validate!(
                ("not", fg(TokenScopeId::KEYWORD_NOT)),
                ("time", fg(TokenScopeId::KEYWORD_TIME)),
                ("--", fg(TokenScopeId::OPTION)),
                ("{", fg(TokenScopeId::KEYWORD)),
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("}", fg(TokenScopeId::KEYWORD)),
            );

            // Overlong paths don't crash (#7837).
            let overlong = get_overlong_path();
            validate!(
                ("touch", fg(TokenScopeId::COMMAND_EXTERNAL)),
                (&overlong, fg(TokenScopeId::PARAM)),
            );

            validate!(
                ("a", fg(TokenScopeId::VARIABLE_DEFINITION_LOCAL_EXPORTED)),
                ("=", fg(TokenScopeId::OPERATOR_ASSIGNMENT), ns),
            );

            // Highlighting works across escaped line breaks (#8444).
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL)),
                ("FISH_\\\n", fg(TokenScopeId::VARIABLE), ns),
                ("VERSION", fg(TokenScopeId::VARIABLE), ns),
            );

            // NOTE: we assume /usr/bin/env exists on the system here
            validate!(
                ("/usr/bin/en", fg(TokenScopeId::COMMAND_EXTERNAL), ns),
                ("*", fg(TokenScopeId::OPERATOR_GLOB), ns)
            );

            validate!(
                ("/usr/bin/e", fg(TokenScopeId::COMMAND_EXTERNAL), ns),
                ("*", fg(TokenScopeId::OPERATOR_GLOB), ns)
            );

            validate!(
                ("/usr/bin/e", fg(TokenScopeId::COMMAND_EXTERNAL), ns),
                ("{", fg(TokenScopeId::OPERATOR_BRACE), ns),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL), ns),
                ("VARIABLE_IN_COMMAND", fg(TokenScopeId::VARIABLE), ns),
                ("}", fg(TokenScopeId::OPERATOR_BRACE), ns),
                ("*", fg(TokenScopeId::OPERATOR_GLOB), ns)
            );

            validate!(
                ("/usr/bin/e", fg(TokenScopeId::COMMAND_EXTERNAL), ns),
                ("$", fg(TokenScopeId::VARIABLE_SIGIL), ns),
                ("VARIABLE_IN_COMMAND2", fg(TokenScopeId::VARIABLE), ns)
            );

            validate!(("$EMPTY_VARIABLE", fg(TokenScopeId::ERROR_COMMAND)));
            validate!(("\"$EMPTY_VARIABLE\"", fg(TokenScopeId::ERROR_COMMAND)));

            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("\\UFDFD", fg(TokenScopeId::ESCAPE_UNICODE)),
            );
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("\\U10FFFF", fg(TokenScopeId::ESCAPE_UNICODE)),
            );
            validate!(
                ("echo", fg(TokenScopeId::BUILTIN)),
                ("\\U110000", fg(TokenScopeId::ESCAPE_INVALID)),
            );

            validate!(
                (">", fg(TokenScopeId::ERROR)),
                ("echo", fg(TokenScopeId::ERROR)),
            );
        });
    }

    /// The enclosing scopes the character that starts `needle` is inside.
    fn enclosing_scopes_of(
        vars: &dyn Environment,
        text: &wstr,
        needle: &wstr,
    ) -> EnclosingScopeSet {
        let mut classifications = vec![];
        highlight_shell(
            text,
            &mut classifications,
            &mut OperationContext::background(vars, EXPANSION_LIMIT_BACKGROUND),
            true, /* io_ok */
            Some(text.len()),
        );

        let offset = text
            .as_char_slice()
            .windows(needle.len())
            .position(|window| window == needle.as_char_slice())
            .expect("the needle is in the text");
        classifications[offset].enclosing()
    }

    /// The classifications of `text`, in a pass that resolves no commands.
    fn classified_without_io(vars: &dyn Environment, text: &wstr) -> Vec<HighlightClassification> {
        let mut classifications = vec![];
        highlight_shell(
            text,
            &mut classifications,
            &mut OperationContext::background(vars, EXPANSION_LIMIT_BACKGROUND),
            false, /* io_ok */
            Some(text.len()),
        );
        classifications
    }

    /// The classifications of `text`, resolving commands and paths.
    fn classified_with_io(vars: &dyn Environment, text: &wstr) -> Vec<HighlightClassification> {
        let mut classifications = vec![];
        highlight_shell(
            text,
            &mut classifications,
            &mut OperationContext::background(vars, EXPANSION_LIMIT_BACKGROUND),
            true, /* io_ok */
            Some(text.len()),
        );
        classifications
    }

    #[test]
    #[serial]
    fn a_function_shadows_the_builtin_of_its_name() {
        test_init();
        let TestParser { ref mut parser, .. } = TestParser::new();

        // A definition is global, so until it is erased no other test may see the name.
        parser.eval(L!("function true; end"), &crate::io::IoChain::new());
        let classifications = classified_with_io(parser.vars(), L!("true"));
        parser.eval(L!("functions --erase true"), &crate::io::IoChain::new());
        assert_eq!(classifications[0].foreground(), TokenScopeId::FUNCTION);
    }

    #[test]
    #[serial]
    fn test_definitions_without_io() {
        test_init();
        let TestParser { ref mut parser, .. } = TestParser::new();
        let vars = parser.vars();

        // `fish_indent`, the pager and an autosuggestion resolve no commands, and the text
        // still declares the mode of the names it defines.
        let classifications = classified_without_io(vars, L!("set -gx name 1"));
        assert_eq!(
            classifications[8].foreground(),
            TokenScopeId::VARIABLE_DEFINITION_GLOBAL_EXPORTED
        );

        // A decorated word is another command, not the builtin that defines.
        let classifications = classified_without_io(vars, L!("command set -gx name 1"));
        assert_eq!(classifications[16].foreground(), TokenScopeId::PARAM);

        // A word the shell reserves names its scope in every pass.
        for (text, expected) in [
            (L!("set -l name 1"), TokenScopeId::BUILTIN_SET),
            (L!("eval echo"), TokenScopeId::BUILTIN_EVAL),
            (L!("read -l name"), TokenScopeId::BUILTIN_READ),
            (L!("test -e /tmp"), TokenScopeId::BUILTIN_TEST),
            (L!("status is-interactive"), TokenScopeId::BUILTIN_STATUS),
            (L!("return 2"), TokenScopeId::KEYWORD_RETURN),
            (L!("break"), TokenScopeId::KEYWORD_BREAK),
            (L!("continue"), TokenScopeId::KEYWORD_CONTINUE),
        ] {
            let classifications = classified_without_io(vars, text);
            assert_eq!(
                classifications[0].foreground(),
                expected,
                "the word at the start of `{text}`"
            );
        }

        // A word the shell does not reserve names the builtin only where it resolved to one,
        // and a pass without I/O resolves no command.
        let classifications = classified_without_io(vars, L!("true"));
        assert_eq!(classifications[0].foreground(), TokenScopeId::COMMAND);
    }

    #[test]
    #[serial]
    fn test_enclosing_scopes() {
        test_init();
        let TestParser { ref mut parser, .. } = TestParser::new();
        let vars = parser.vars();

        let only = |scope| EnclosingScopeSet::default().with_scope(scope);
        let block = only(TokenScopeId::META_BLOCK);
        let r#if = only(TokenScopeId::META_IF);
        let case = only(TokenScopeId::META_CASE);
        let function = only(TokenScopeId::META_FUNCTION);
        let no_shadowing = function.with_scope(TokenScopeId::META_FUNCTION_NO_SHADOWING);
        let loop_for = only(TokenScopeId::META_LOOP).with_scope(TokenScopeId::META_LOOP_FOR);
        let loop_while = only(TokenScopeId::META_LOOP).with_scope(TokenScopeId::META_LOOP_WHILE);
        let pipeline = only(TokenScopeId::META_PIPELINE);
        let and_or = only(TokenScopeId::META_AND_OR);
        let command_substitution = only(TokenScopeId::META_COMMAND_SUBSTITUTION);

        let cases: &[(&wstr, &wstr, EnclosingScopeSet)] = &[
            // The construct that owns the region marks it, not the keyword that opens it.
            (
                L!("begin echo a; end"),
                L!("begin"),
                EnclosingScopeSet::default(),
            ),
            (L!("begin echo a; end"), L!("echo"), block),
            // A body nests inside what encloses it.
            (
                L!("function f; begin echo a; end; end"),
                L!("echo"),
                function | block,
            ),
            (
                L!("if true; echo b; end"),
                L!("true"),
                EnclosingScopeSet::default(),
            ),
            (L!("if true; echo b; end"), L!("echo"), r#if),
            (L!("switch x; case a; echo b; end"), L!("echo"), case),
            (L!("for x in a; echo b; end"), L!("echo"), loop_for),
            (L!("while true; echo b; end"), L!("echo"), loop_while),
            (L!("function f; echo b; end"), L!("echo"), function),
            (L!("function f -S; echo b; end"), L!("echo"), no_shadowing),
            // A word that an option takes as its value is not an option.
            (
                L!("function f --description -S; echo b; end"),
                L!("echo"),
                function,
            ),
            (
                L!("function f --description d -S; echo b; end"),
                L!("echo"),
                no_shadowing,
            ),
            (L!("echo a | cat"), L!("cat"), pipeline),
            (L!("true; and false"), L!("false"), and_or),
            // An and-or list with no conjunction and a pipeline with no pipe own no region.
            (
                L!("true; and false"),
                L!("true"),
                EnclosingScopeSet::default(),
            ),
            (L!("echo a"), L!("echo"), EnclosingScopeSet::default()),
            (L!("echo $(true)"), L!("true"), command_substitution),
        ];

        for (text, needle, expected) in cases {
            assert_eq!(
                enclosing_scopes_of(vars, text, needle),
                *expected,
                "the enclosing scopes of `{needle}` in `{text}`"
            );
        }
    }
}
