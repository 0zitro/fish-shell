//! Implementation of the `suspend` builtin.

use super::prelude::*;
use crate::fds::{heightenize_fd, make_autoclose_pipes, wopen_cloexec};
use crate::io::{IoChain, IoFile, IoPipe};
use crate::parse_constants::{ParseErrorList, ParseTreeFlags};
use crate::parse_tree::parse_source;
use crate::proc::{
    SuspensionDirection, SuspensionForm, SuspensionRecord, get_login, is_interactive_session,
};
use crate::signal::signal_check_cancel;
use crate::wutil::perror_nix;
use fish_wcstringutil::join_strings;
use fish_widestring::{WExt as _, WString};
use libc::{F_GETFD, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::fcntl::OFlag;
use nix::pty::openpty;
use nix::sys::signal::{Signal, kill};
use nix::sys::stat::Mode;
use nix::unistd::{dup, getpid};
use std::os::fd::{AsFd as _, FromRawFd as _, OwnedFd, RawFd};
use std::os::unix::net::UnixStream;
use std::sync::Arc;

/// The descriptor layout a `--fd` binding creates.
enum BindingBackend {
    Pipe,
    Inherit {
        path: WString,
    },
    Pty,
    Socketpair,
    Pair {
        source_descriptor: RawFd,
        destination_descriptor: RawFd,
    },
}

impl BindingBackend {
    /// The direction a binding of this backend takes when the specification omits one.
    fn default_direction(&self) -> SuspensionDirection {
        match self {
            BindingBackend::Pipe => SuspensionDirection::Out,
            BindingBackend::Inherit { .. }
            | BindingBackend::Pty
            | BindingBackend::Socketpair
            | BindingBackend::Pair { .. } => SuspensionDirection::InOut,
        }
    }
}

/// One descriptor beyond the standard ones that the command form suspends.
struct FdBinding {
    /// The descriptor of the suspended pipeline.
    fd: RawFd,
    /// The direction of that descriptor in the suspended pipeline.
    direction: SuspensionDirection,
    /// The backend that creates or opens the descriptor.
    backend: BindingBackend,
}

/// Why a `--fd` specification is not accepted.
pub enum FdBindingError {
    /// The specification does not match `FD[:DIRECTION][=BACKEND]`.
    Malformed,
    /// The descriptor is one of 0, 1 or 2, which the command form suspends already.
    StandardDescriptor,
    /// A pipe binding names `inout`.
    InoutWithoutBackend,
    /// The backend is not one this layer accepts.
    UnknownBackend,
    /// A pair descriptor is not a distinct open descriptor above 2.
    InvalidPairDescriptor,
    /// The backend does not accept the direction.
    DirectionUnsupported,
}

impl TryFrom<&wstr> for SuspensionDirection {
    type Error = FdBindingError;

    fn try_from(text: &wstr) -> Result<Self, Self::Error> {
        match text {
            text if text == L!("in") => Ok(SuspensionDirection::In),
            text if text == L!("out") => Ok(SuspensionDirection::Out),
            text if text == L!("inout") => Ok(SuspensionDirection::InOut),
            _ => Err(FdBindingError::Malformed),
        }
    }
}

/// Parse one `--fd` specification, `FD[:DIRECTION][=BACKEND]`.
fn parse_fd_binding(specification: &wstr) -> Result<FdBinding, FdBindingError> {
    let (descriptor_text, backend_text) = match specification.find_char('=') {
        Some(equals) => (
            specification.slice_to(equals),
            Some(specification.slice_from(equals + 1)),
        ),
        None => (specification, None),
    };

    let (fd_text, direction_text) = match descriptor_text.find_char(':') {
        Some(colon) => (
            descriptor_text.slice_to(colon),
            Some(descriptor_text.slice_from(colon + 1)),
        ),
        None => (descriptor_text, None),
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

    let backend = match backend_text {
        None => BindingBackend::Pipe,
        Some(backend_text) => parse_binding_backend(backend_text)?,
    };

    let direction = match direction_text {
        None => backend.default_direction(),
        Some(text) => SuspensionDirection::try_from(text)?,
    };

    match &backend {
        BindingBackend::Pipe => {
            if direction == SuspensionDirection::InOut {
                return Err(FdBindingError::InoutWithoutBackend);
            }
        }
        BindingBackend::Pty | BindingBackend::Socketpair => {
            if direction != SuspensionDirection::InOut {
                return Err(FdBindingError::DirectionUnsupported);
            }
        }
        BindingBackend::Inherit { .. } | BindingBackend::Pair { .. } => {}
    }

    Ok(FdBinding {
        fd,
        direction,
        backend,
    })
}

/// Parse the backend part of a `--fd` specification.
fn parse_binding_backend(text: &wstr) -> Result<BindingBackend, FdBindingError> {
    if text == L!("pty") {
        return Ok(BindingBackend::Pty);
    }
    if text == L!("socketpair") {
        return Ok(BindingBackend::Socketpair);
    }
    if let Some(descriptors) = text.strip_prefix(L!("pair:")) {
        return parse_pair_backend(descriptors);
    }
    let Some(path) = text.strip_prefix(L!("inherit:")) else {
        return Err(FdBindingError::UnknownBackend);
    };
    if path.is_empty() {
        return Err(FdBindingError::Malformed);
    }
    Ok(BindingBackend::Inherit {
        path: path.to_owned(),
    })
}

/// Return whether `fd` names an open descriptor.
///
/// A user-supplied raw descriptor may be closed, so it cannot be wrapped in a
/// `BorrowedFd`: that type asserts validity for its lifetime. `fcntl(F_GETFD)`
/// inspects the descriptor table without dereferencing memory and reports
/// `EBADF` for a closed descriptor, so it is the safe operation for this check.
fn descriptor_is_open(fd: RawFd) -> bool {
    // SAFETY: `fcntl(F_GETFD)` accepts any non-negative descriptor number and
    // only reads the descriptor table; it dereferences no Rust value.
    unsafe { libc::fcntl(fd, F_GETFD) >= 0 }
}

/// Parse the two descriptor numbers of a `pair:S:D` backend.
fn parse_pair_backend(text: &wstr) -> Result<BindingBackend, FdBindingError> {
    let Some(colon) = text.find_char(':') else {
        return Err(FdBindingError::InvalidPairDescriptor);
    };
    let Ok(source_descriptor) = fish_wcstol(text.slice_to(colon)) else {
        return Err(FdBindingError::InvalidPairDescriptor);
    };
    let Ok(destination_descriptor) = fish_wcstol(text.slice_from(colon + 1)) else {
        return Err(FdBindingError::InvalidPairDescriptor);
    };
    let (Ok(source_descriptor), Ok(destination_descriptor)) = (
        RawFd::try_from(source_descriptor),
        RawFd::try_from(destination_descriptor),
    ) else {
        return Err(FdBindingError::InvalidPairDescriptor);
    };
    if source_descriptor <= STDERR_FILENO
        || destination_descriptor <= STDERR_FILENO
        || source_descriptor == destination_descriptor
        || !descriptor_is_open(source_descriptor)
        || !descriptor_is_open(destination_descriptor)
    {
        return Err(FdBindingError::InvalidPairDescriptor);
    }
    Ok(BindingBackend::Pair {
        source_descriptor,
        destination_descriptor,
    })
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
                            "suspend: %s: expected FD[:in|out|inout][=BACKEND]",
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
                    Err(FdBindingError::InoutWithoutBackend) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: inout requires a backend",
                            specification
                        ));
                        return Err(STATUS_INVALID_ARGS);
                    }
                    Err(FdBindingError::UnknownBackend) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: unknown backend",
                            specification
                        ));
                        return Err(STATUS_INVALID_ARGS);
                    }
                    Err(FdBindingError::InvalidPairDescriptor) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: invalid pair descriptor",
                            specification
                        ));
                        return Err(STATUS_INVALID_ARGS);
                    }
                    Err(FdBindingError::DirectionUnsupported) => {
                        streams.err.appendln(&wgettext_fmt!(
                            "suspend: %s: direction unsupported by backend",
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
        let suspension = SuspensionRecord::new(SuspensionForm::Stream);
        suspension.record_descriptor(STDIN_FILENO, SuspensionDirection::Out, shell_end);
        let handle = parser.add_suspension(suspension);
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

    let suspension = SuspensionRecord::new(SuspensionForm::Command);
    suspension.record_descriptor(STDIN_FILENO, SuspensionDirection::In, stdin_pipe.write);
    suspension.record_descriptor(STDOUT_FILENO, SuspensionDirection::Out, stdout_pipe.read);
    suspension.record_descriptor(STDERR_FILENO, SuspensionDirection::Out, stderr_pipe.read);

    for binding in bindings {
        match &binding.backend {
            BindingBackend::Pipe => {
                let Ok(pipes) = make_autoclose_pipes() else {
                    perror_nix("pipe", nix::Error::last());
                    return Err(STATUS_CMD_ERROR);
                };
                match binding.direction {
                    SuspensionDirection::Out => {
                        io_chain.push(Arc::new(IoPipe::new(binding.fd, false, pipes.write)));
                        suspension.record_descriptor(
                            binding.fd,
                            SuspensionDirection::Out,
                            pipes.read,
                        );
                    }
                    SuspensionDirection::In => {
                        io_chain.push(Arc::new(IoPipe::new(binding.fd, true, pipes.read)));
                        suspension.record_descriptor(
                            binding.fd,
                            SuspensionDirection::In,
                            pipes.write,
                        );
                    }
                    SuspensionDirection::InOut => panic!("pipe binding rejects inout"),
                }
            }
            BindingBackend::Inherit { path } => {
                let flags = match binding.direction {
                    SuspensionDirection::In => OFlag::O_RDONLY,
                    SuspensionDirection::Out => OFlag::O_WRONLY,
                    SuspensionDirection::InOut => OFlag::O_RDWR,
                };
                let Ok(file) = wopen_cloexec(path, flags, Mode::empty()) else {
                    streams
                        .err
                        .appendln(&wgettext_fmt!("suspend: %s: cannot open", &path));
                    return Err(STATUS_CMD_ERROR);
                };
                io_chain.push(Arc::new(IoFile::new(binding.fd, file)));
            }
            BindingBackend::Pty => {
                let Ok(pty) = openpty(None, None) else {
                    perror_nix("openpty", nix::Error::last());
                    return Err(STATUS_CMD_ERROR);
                };
                let Ok(slave) = heightenize_fd(pty.slave, false) else {
                    return Err(STATUS_CMD_ERROR);
                };
                let Ok(master) = heightenize_fd(pty.master, false) else {
                    return Err(STATUS_CMD_ERROR);
                };
                io_chain.push(Arc::new(IoPipe::new(binding.fd, false, slave)));
                suspension.record_descriptor(binding.fd, SuspensionDirection::InOut, master);
            }
            BindingBackend::Socketpair => {
                let Ok((source_endpoint, record_endpoint)) = UnixStream::pair() else {
                    perror_nix("socketpair", nix::Error::last());
                    return Err(STATUS_CMD_ERROR);
                };
                let Ok(source_endpoint) = heightenize_fd(OwnedFd::from(source_endpoint), true)
                else {
                    return Err(STATUS_CMD_ERROR);
                };
                let Ok(record_endpoint) = heightenize_fd(OwnedFd::from(record_endpoint), true)
                else {
                    return Err(STATUS_CMD_ERROR);
                };
                io_chain.push(Arc::new(IoPipe::new(binding.fd, false, source_endpoint)));
                suspension.record_descriptor(
                    binding.fd,
                    SuspensionDirection::InOut,
                    record_endpoint,
                );
            }
            BindingBackend::Pair {
                source_descriptor,
                destination_descriptor,
            } => {
                // SAFETY: `parse_pair_backend` verified that the descriptor is
                // open; the binding takes ownership of it as the source endpoint
                // so the peer can observe end of file when the source exits.
                let Ok(source_endpoint) =
                    heightenize_fd(unsafe { OwnedFd::from_raw_fd(*source_descriptor) }, false)
                else {
                    return Err(STATUS_CMD_ERROR);
                };
                // SAFETY: `parse_pair_backend` verified that the descriptor is
                // open; the record takes ownership of it and closes it on drop.
                let Ok(destination_endpoint) = heightenize_fd(
                    unsafe { OwnedFd::from_raw_fd(*destination_descriptor) },
                    false,
                ) else {
                    return Err(STATUS_CMD_ERROR);
                };
                io_chain.push(Arc::new(IoPipe::new(
                    binding.fd,
                    binding.direction == SuspensionDirection::In,
                    source_endpoint,
                )));
                suspension.record_descriptor(binding.fd, binding.direction, destination_endpoint);
            }
        }
    }

    let eval_res = parser.eval_parsed_source_background(&parsed_source, &io_chain);
    // The source stages are tasks of the background job; run them to their first blocking point
    // before the handle is written, so a later resume finds the recorded stream producing.
    parser.run_runnable_tasks();
    let cancel_signal = signal_check_cancel();
    if cancel_signal != 0 {
        return Err(128 + cancel_signal);
    }
    if eval_res.status.signal_exited() {
        return BuiltinResult::from_dynamic(eval_res.status.status_value());
    }
    if eval_res.break_expand {
        return Err(STATUS_CMD_ERROR);
    }
    let handle = parser.add_suspension(suspension);
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
