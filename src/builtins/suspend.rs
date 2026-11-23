//! Implementation of the `suspend` builtin.

use super::prelude::*;
use crate::fds::make_autoclose_pipes;
use crate::io::{IoChain, IoPipe};
use crate::parse_constants::{ParseErrorList, ParseTreeFlags};
use crate::parse_tree::parse_source;
use crate::proc::{
    Suspension, SuspensionDirection, SuspensionForm, get_login, is_interactive_session,
};
use crate::wutil::perror_nix;
use fish_wcstringutil::join_strings;
use fish_widestring::WExt as _;
use libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::sys::signal::{Signal, kill};
use nix::unistd::{dup, getpid};
use std::os::fd::{AsFd as _, RawFd};
use std::sync::Arc;

/// One descriptor beyond the standard ones that the command form suspends.
#[derive(Clone, Copy)]
struct FdBinding {
    /// The descriptor of the suspended pipeline.
    fd: RawFd,
    /// The side of the pipe the suspended pipeline uses.
    direction: SuspensionDirection,
}

/// Why a `--fd` specification is not accepted.
enum FdBindingError {
    /// The specification is not `FD:in` or `FD:out`.
    Malformed,
    /// The descriptor is one of 0, 1 or 2, which the command form suspends already.
    StandardDescriptor,
}

/// Parse one `--fd` specification, `FD`, `FD:in` or `FD:out`. The direction defaults to `out`.
fn parse_fd_binding(specification: &wstr) -> Result<FdBinding, FdBindingError> {
    let (fd_text, direction_text) = match specification.find_char(':') {
        Some(colon) => (
            specification.slice_to(colon),
            specification.slice_from(colon + 1),
        ),
        None => (specification, L!("out")),
    };
    let Ok(fd) = fish_wcstol(fd_text) else {
        return Err(FdBindingError::Malformed);
    };
    let Ok(fd) = RawFd::try_from(fd) else {
        return Err(FdBindingError::Malformed);
    };
    if fd <= STDERR_FILENO {
        return Err(FdBindingError::StandardDescriptor);
    }
    let direction = if direction_text == L!("in") {
        SuspensionDirection::In
    } else if direction_text == L!("out") {
        SuspensionDirection::Out
    } else {
        return Err(FdBindingError::Malformed);
    };
    Ok(FdBinding { fd, direction })
}

/// Suspend the current shell, a stream on its standard input, or a command pipeline.
///
/// The shell stops itself with `SIGSTOP` rather than `SIGTSTP`: an interactive shell ignores
/// `SIGTSTP`, and the stop is what returns control to the controlling process. `SIGCONT` resumes
/// the shell.
///
/// When standard input is directly redirected, the command records the shell's own copy of the
/// input descriptor as a suspension and writes the handle that denotes it. The input is the stream
/// a pipe delivers, whatever stage follows.
///
/// When operands are present, the joined source is one pipeline; the command starts it with
/// descriptors 0, 1 and 2 replaced by pipes whose shell ends are recorded, and writes the handle.
pub fn suspend(parser: &mut Parser, streams: &mut IoStreams, argv: &mut [&wstr]) -> BuiltinResult {
    let cmd = argv[0];

    let shortopts: &wstr = L!("+fh");
    let longopts: &[WOption] = &[
        wopt(L!("fd"), ArgType::RequiredArgument, 'd'),
        wopt(L!("force"), ArgType::NoArgument, 'f'),
        wopt(L!("help"), ArgType::NoArgument, 'h'),
    ];

    let mut force = false;
    let mut print_help = false;
    let mut bindings: Vec<FdBinding> = Vec::new();
    let mut w = WGetopter::new(shortopts, longopts, argv);
    while let Some(c) = w.next_opt() {
        match c {
            'd' => {
                let specification = w.woptarg.expect("WGetopter provides the argument of --fd");
                match parse_fd_binding(specification) {
                    Ok(binding) => {
                        if bindings.iter().any(|existing| existing.fd == binding.fd) {
                            streams.err.appendln(&wgettext_fmt!(
                                "suspend: %s: descriptor bound twice",
                                specification
                            ));
                            return Err(STATUS_INVALID_ARGS);
                        }
                        bindings.push(binding);
                    }
                    Err(FdBindingError::Malformed) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: expected FD, FD:in or FD:out",
                            specification
                        ));
                        return Err(STATUS_INVALID_ARGS);
                    }
                    Err(FdBindingError::StandardDescriptor) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: standard descriptors are suspended already",
                            specification
                        ));
                        return Err(STATUS_INVALID_ARGS);
                    }
                }
            }
            'f' => {
                force = true;
            }
            'h' => {
                print_help = true;
            }
            ':' => {
                builtin_missing_argument(parser, streams, cmd, None, argv[w.wopt_index - 1], true);
                return Err(STATUS_INVALID_ARGS);
            }
            ';' => {
                builtin_unexpected_argument(parser, streams, cmd, argv[w.wopt_index - 1], true);
                return Err(STATUS_INVALID_ARGS);
            }
            '?' => {
                builtin_unknown_option(parser, streams, cmd, argv[w.wopt_index - 1], true);
                return Err(STATUS_INVALID_ARGS);
            }
            _ => {
                panic!("unexpected retval from WGetopter");
            }
        }
    }

    if print_help {
        builtin_print_help(parser, streams, cmd);
        return Ok(SUCCESS);
    }

    let source_words = &w.argv[w.wopt_index..];
    if !source_words.is_empty() {
        if streams.stdin_is_directly_redirected {
            streams.err.appendln(wgettext!(
                "suspend: the command form is not a pipeline stage"
            ));
            return Err(STATUS_INVALID_ARGS);
        }
        return suspend_command_pipeline(parser, streams, source_words, &bindings);
    }
    if !bindings.is_empty() {
        streams
            .err
            .appendln(wgettext!("suspend: --fd requires a source pipeline"));
        return Err(STATUS_INVALID_ARGS);
    }

    if streams.stdin_is_directly_redirected {
        let Some(stdin_file) = streams.stdin_file.as_ref() else {
            streams
                .err
                .appendln(wgettext!("Cannot suspend a closed standard input"));
            return Err(STATUS_CMD_ERROR);
        };
        let shell_end = match dup(stdin_file.as_fd()) {
            Ok(shell_end) => shell_end,
            Err(err) => {
                perror_nix("dup", err);
                return Err(STATUS_CMD_ERROR);
            }
        };
        let mut suspension = Suspension::new(SuspensionForm::Stream);
        suspension.record_descriptor(STDIN_FILENO, SuspensionDirection::Out, shell_end);
        let handle = suspension.handle;
        parser.add_suspension(suspension);
        streams.out.appendln(&handle.to_wstring());
        return Ok(SUCCESS);
    }

    if !force && get_login() && is_interactive_session() {
        streams
            .err
            .appendln(wgettext!("Refusing to suspend interactive login shell."));
        streams.err.appendln(wgettext!(
            "Use --force to override. This might hang your terminal."
        ));
        return Err(STATUS_CMD_ERROR);
    }

    let shell_pid = getpid();
    if is_interactive_session() {
        streams.out.appendln(&wgettext_fmt!(
            "Suspending %d: run kill -CONT %d from another terminal to resume",
            shell_pid.as_raw(),
            shell_pid.as_raw()
        ));
    }

    match kill(shell_pid, Signal::SIGSTOP) {
        Ok(()) => Ok(SUCCESS),
        Err(err) => {
            perror_nix("kill", err);
            Err(STATUS_CMD_ERROR)
        }
    }
}

/// Start the joined source as one pipeline with descriptors 0, 1 and 2 suspended and every `--fd`
/// binding suspended besides.
///
/// The source is one pipeline, so the job it starts is the unit whose descriptors the record
/// describes. Its stdin is fed by the shell end of one pipe; its stdout and stderr write into the
/// other ends of two pipes. A binding adds one more pipe for its descriptor. The handle is written
/// after the job starts.
fn suspend_command_pipeline(
    parser: &mut Parser,
    streams: &mut IoStreams,
    source_words: &[&wstr],
    bindings: &[FdBinding],
) -> BuiltinResult {
    let source = join_strings(source_words, ' ');

    let mut errors = ParseErrorList::new();
    let Some(parsed_source) =
        parse_source(source.clone(), ParseTreeFlags::default(), Some(&mut errors))
    else {
        eprintf!("%s", parser.get_backtrace(&source, &errors));
        return Err(STATUS_INVALID_ARGS);
    };

    let job_list = parsed_source.top_job_list();
    let mut conjunctions = job_list.iter();
    let first_conjunction = conjunctions.next();
    if conjunctions.next().is_some() {
        return report_source_not_one_pipeline(streams);
    }
    let Some(conjunction) = first_conjunction else {
        return report_source_not_one_pipeline(streams);
    };
    if !conjunction.continuations.is_empty() || conjunction.job.bg.is_some() {
        return report_source_not_one_pipeline(streams);
    }

    let Ok(stdin_pipe) = make_autoclose_pipes() else {
        perror_nix("pipe", nix::Error::last());
        return Err(STATUS_CMD_ERROR);
    };
    let Ok(stdout_pipe) = make_autoclose_pipes() else {
        perror_nix("pipe", nix::Error::last());
        return Err(STATUS_CMD_ERROR);
    };
    let Ok(stderr_pipe) = make_autoclose_pipes() else {
        perror_nix("pipe", nix::Error::last());
        return Err(STATUS_CMD_ERROR);
    };

    let mut io_chain = IoChain::new();
    io_chain.push(Arc::new(IoPipe::new(STDIN_FILENO, true, stdin_pipe.read)));
    io_chain.push(Arc::new(IoPipe::new(
        STDOUT_FILENO,
        false,
        stdout_pipe.write,
    )));
    io_chain.push(Arc::new(IoPipe::new(
        STDERR_FILENO,
        false,
        stderr_pipe.write,
    )));

    let mut suspension = Suspension::new(SuspensionForm::Command);
    suspension.record_descriptor(STDIN_FILENO, SuspensionDirection::In, stdin_pipe.write);
    suspension.record_descriptor(STDOUT_FILENO, SuspensionDirection::Out, stdout_pipe.read);
    suspension.record_descriptor(STDERR_FILENO, SuspensionDirection::Out, stderr_pipe.read);

    for binding in bindings {
        let Ok(pipes) = make_autoclose_pipes() else {
            perror_nix("pipe", nix::Error::last());
            return Err(STATUS_CMD_ERROR);
        };
        match binding.direction {
            SuspensionDirection::Out => {
                io_chain.push(Arc::new(IoPipe::new(binding.fd, false, pipes.write)));
                suspension.record_descriptor(binding.fd, SuspensionDirection::Out, pipes.read);
            }
            SuspensionDirection::In => {
                io_chain.push(Arc::new(IoPipe::new(binding.fd, true, pipes.read)));
                suspension.record_descriptor(binding.fd, SuspensionDirection::In, pipes.write);
            }
        }
    }

    let handle = suspension.handle;

    let eval_res = parser.eval_parsed_source_background(&parsed_source, &io_chain);
    if eval_res.status.signal_exited() {
        return BuiltinResult::from_dynamic(eval_res.status.status_value());
    }
    if eval_res.break_expand {
        return Err(STATUS_CMD_ERROR);
    }
    parser.add_suspension(suspension);
    streams.out.appendln(&handle.to_wstring());
    Ok(SUCCESS)
}

/// Report that the joined source is not one pipeline.
fn report_source_not_one_pipeline(streams: &mut IoStreams) -> BuiltinResult {
    streams.err.appendln(wgettext!(
        "suspend: the joined source is not one foreground pipeline"
    ));
    Err(STATUS_INVALID_ARGS)
}
