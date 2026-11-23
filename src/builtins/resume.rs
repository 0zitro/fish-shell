//! Implementation of the `resume` builtin.

use super::prelude::*;
use crate::fds::make_autoclose_pipes;
use crate::proc::{
    SuspensionDirection, SuspensionForm, SuspensionHandle, SuspensionRecord, SuspensionResolution,
};
use crate::wutil::perror_nix;
use fish_widestring::bytes2wcstring;
use libc::{POLLHUP, POLLIN, SHUT_WR, STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::unistd::{dup, read};
use std::os::fd::{AsFd as _, AsRawFd as _, BorrowedFd, OwnedFd, RawFd};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::mpsc::{Sender, channel};
use std::thread::{self, ScopedJoinHandle};

/// The role a stdin target plays in the suspended command.
#[derive(Clone, Copy, Eq, PartialEq)]
enum StdinTargetKind {
    /// Descriptor 0 of the command-form source.
    SourceStandardInput,
    /// A duplex descriptor recorded by `--fd`.
    DuplexEndpoint,
}

/// One suspension descriptor that receives the resuming command's standard input.
struct StdinTarget {
    descriptor: Arc<OwnedFd>,
    kind: StdinTargetKind,
}

/// Return whether stdin is ready, or `false` when the completion pipe is ready.
///
/// SAFETY: `poll` only inspects the descriptor table and writes the `revents`
/// fields of a local `pollfd` array; no Rust value invariant is assumed.
fn stdin_or_completion_ready(stdin_fd: RawFd, completion_fd: RawFd) -> bool {
    let mut pollfds = [
        libc::pollfd {
            fd: stdin_fd,
            events: POLLIN,
            revents: 0,
        },
        libc::pollfd {
            fd: completion_fd,
            events: POLLIN,
            revents: 0,
        },
    ];
    loop {
        let result = unsafe { libc::poll(pollfds.as_mut_ptr(), 2, -1) };
        if result < 0 {
            if nix::Error::last() == nix::Error::EINTR {
                continue;
            }
            return false;
        }
        return pollfds[0].revents & (POLLIN | POLLHUP) != 0;
    }
}

/// Consume a suspension and connect its descriptors to the command's own descriptors.
///
/// A recorded descriptor is a pipe end the suspended pipeline used. The command form recorded which
/// side the pipeline used: an `Out` descriptor is the shell's read end, relayed to the command's
/// descriptor of the same number; an `In` descriptor is the shell's write end, fed from the
/// command's descriptor of the same number. A stream suspension records only descriptor 0, whose
/// stream the command writes to its standard output.
pub fn resume(parser: &mut Parser, streams: &mut IoStreams, argv: &mut [&wstr]) -> BuiltinResult {
    let cmd = argv[0];

    let shortopts: &wstr = L!("h");
    let longopts: &[WOption] = &[wopt(L!("help"), ArgType::NoArgument, 'h')];

    let mut print_help = false;
    let mut w = WGetopter::new(shortopts, longopts, argv);
    while let Some(c) = w.next_opt() {
        match c {
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

    let operands = &w.argv[w.wopt_index..];
    if operands.len() != 1 {
        streams
            .err
            .appendln(wgettext!("resume: expected one suspension handle"));
        return Err(STATUS_INVALID_ARGS);
    }
    let Some(handle) = SuspensionHandle::parse(operands[0]) else {
        streams.err.appendln(&wgettext_fmt!(
            "resume: %s is not a suspension handle",
            operands[0]
        ));
        return Err(STATUS_INVALID_ARGS);
    };
    let suspension = match parser.consume_suspension(handle) {
        SuspensionResolution::Unknown => {
            streams.err.appendln(&wgettext_fmt!(
                "resume: %s denotes no suspension",
                operands[0]
            ));
            return Err(STATUS_CMD_ERROR);
        }
        SuspensionResolution::Consumed => {
            streams.err.appendln(&wgettext_fmt!(
                "resume: %s: the suspension is consumed",
                operands[0]
            ));
            return Err(STATUS_CMD_ERROR);
        }
        SuspensionResolution::Available(record) => record,
    };

    if streams.out.flush_and_check_error() != STATUS_CMD_OK {
        return Err(STATUS_CMD_ERROR);
    }
    match suspension.form {
        SuspensionForm::Stream => resume_stream(streams, &suspension),
        SuspensionForm::Command => resume_command(streams, &suspension),
    }
}

/// Link a stream suspension's descriptor to the command's standard output.
fn resume_stream(streams: &mut IoStreams, suspension: &SuspensionRecord) -> BuiltinResult {
    let Some((direction, suspended_stream)) = suspension.take_descriptor(STDIN_FILENO) else {
        streams
            .err
            .appendln(wgettext!("resume: the suspension records no stream"));
        return Err(STATUS_CMD_ERROR);
    };
    if direction != SuspensionDirection::Out {
        streams
            .err
            .appendln(wgettext!("resume: the suspension records no stream"));
        return Err(STATUS_CMD_ERROR);
    }

    let (sender, receiver) = channel::<(bool, Vec<u8>)>();
    let result = thread::scope(|scope| {
        let sink = stdout_sink(streams);
        let relay = scope.spawn(move || relay_to_sink(suspended_stream, sink, false, sender));
        let received = receive_output(streams, &receiver);
        let joined = match relay.join() {
            Ok(result) => result,
            Err(_) => Err(nix::Error::EIO),
        };
        received.and(joined)
    });
    result.map_err(|err| {
        if err != nix::Error::EPIPE {
            perror_nix("resume", err);
        }
        STATUS_CMD_ERROR
    })?;
    finish_outputs(streams)
}

/// Link a command suspension's descriptors to the command's own descriptors.
fn resume_command(streams: &mut IoStreams, suspension: &SuspensionRecord) -> BuiltinResult {
    let descriptors = suspension.take_all_descriptors();
    let has_duplex = descriptors
        .values()
        .any(|(direction, _)| *direction == SuspensionDirection::InOut);
    let duplex_output_count = if streams.stdin_file.is_some() {
        descriptors
            .values()
            .filter(|(direction, _)| *direction == SuspensionDirection::InOut)
            .count()
    } else {
        0
    };
    let remaining_duplex_outputs = Arc::new(AtomicUsize::new(duplex_output_count));
    let completion_pipe = if has_duplex && streams.stdin_file.is_some() {
        let Ok(pipes) = make_autoclose_pipes() else {
            perror_nix("pipe", nix::Error::last());
            return Err(STATUS_CMD_ERROR);
        };
        Some(pipes)
    } else {
        None
    };
    let (completion_read, completion_write) = match completion_pipe {
        Some(pipes) => (Some(pipes.read), Some(pipes.write)),
        None => (None, None),
    };
    let (sender, receiver) = channel::<(bool, Vec<u8>)>();
    let result = thread::scope(|scope| {
        let mut relays: Vec<ScopedJoinHandle<'_, Result<(), nix::Error>>> = Vec::new();
        let mut stdin_targets: Vec<StdinTarget> = Vec::new();

        for (descriptor, (direction, stream)) in descriptors {
            match direction {
                SuspensionDirection::In => {
                    if descriptor == STDIN_FILENO {
                        if streams.stdin_is_directly_redirected {
                            let Some(stdin_file) = streams.stdin_file.as_ref() else {
                                continue;
                            };
                            if has_duplex {
                                stdin_targets.push(StdinTarget {
                                    descriptor: stream,
                                    kind: StdinTargetKind::SourceStandardInput,
                                });
                            } else {
                                match dup(stdin_file.as_fd()) {
                                    Ok(source) => {
                                        relays.push(
                                            scope.spawn(move || {
                                                relay(source.as_fd(), stream.as_fd())
                                            }),
                                        );
                                    }
                                    Err(err) => {
                                        perror_nix("dup", err);
                                        return Err(err);
                                    }
                                }
                            }
                        }
                        // Otherwise the feed end drops here and the pipeline reads end of file.
                    } else {
                        let Some(source_fd) = streams.io_chain.resolve_fd(descriptor) else {
                            continue;
                        };
                        relays.push(scope.spawn(move || {
                            // The source descriptor belongs to the command's io chain and outlives
                            // this call.
                            let source = unsafe { BorrowedFd::borrow_raw(source_fd) };
                            relay(source, stream.as_fd())
                        }));
                    }
                }
                SuspensionDirection::Out => {
                    let sink = descriptor_sink(streams, descriptor);
                    let is_stderr = descriptor == STDERR_FILENO;
                    let sender = sender.clone();
                    relays
                        .push(scope.spawn(move || relay_to_sink(stream, sink, is_stderr, sender)));
                }
                SuspensionDirection::InOut => {
                    let Some(stdin_file) = streams.stdin_file.as_ref() else {
                        // SAFETY: `shutdown` only inspects the descriptor table;
                        // the descriptor belongs to this record and is valid here.
                        let _ = unsafe { libc::shutdown(stream.as_raw_fd(), SHUT_WR) };
                        drop(stream);
                        continue;
                    };
                    let _ = stdin_file;
                    let Ok(recorded_output) = dup(stream.as_fd()) else {
                        perror_nix("dup", nix::Error::last());
                        return Err(nix::Error::last());
                    };

                    stdin_targets.push(StdinTarget {
                        descriptor: stream,
                        kind: StdinTargetKind::DuplexEndpoint,
                    });
                    let output_sender = sender.clone();
                    let sink = stdout_sink(streams);
                    let remaining_duplex_outputs = Arc::clone(&remaining_duplex_outputs);
                    let completion = match completion_write.as_ref() {
                        Some(completion_write) => match dup(completion_write.as_fd()) {
                            Ok(completion) => Some(completion),
                            Err(err) => {
                                perror_nix("dup", err);
                                return Err(err);
                            }
                        },
                        None => None,
                    };
                    relays.push(scope.spawn(move || {
                        let result = match relay_to_sink(
                            Arc::new(recorded_output),
                            sink,
                            false,
                            output_sender,
                        ) {
                            Err(nix::Error::EIO) => Ok(()),
                            result => result,
                        };
                        if remaining_duplex_outputs.fetch_sub(1, Ordering::Relaxed) == 1 {
                            if let Some(completion) = completion {
                                let _ = write_all(completion.as_fd(), &[1]);
                            }
                        }
                        result
                    }));
                }
            }
        }
        drop(sender);

        let stdin_thread = if stdin_targets.is_empty() {
            None
        } else {
            let Some(stdin_file) = streams.stdin_file.as_ref() else {
                return Err(nix::Error::EBADF);
            };
            let Ok(stdin_descriptor) = dup(stdin_file.as_fd()) else {
                perror_nix("dup", nix::Error::last());
                return Err(nix::Error::last());
            };
            let Some(completion_read) = completion_read else {
                return Err(nix::Error::EBADF);
            };
            Some(scope.spawn(move || {
                let mut buffer = [0u8; 4096];
                loop {
                    if !stdin_or_completion_ready(
                        stdin_descriptor.as_raw_fd(),
                        completion_read.as_raw_fd(),
                    ) {
                        break;
                    }
                    match read(stdin_descriptor.as_fd(), &mut buffer) {
                        Ok(0) => break,
                        Ok(count) => {
                            for target in &stdin_targets {
                                write_all(target.descriptor.as_fd(), &buffer[..count])?;
                            }
                        }
                        Err(nix::Error::EINTR) => continue,
                        Err(err) => return Err(err),
                    }
                }
                for target in &stdin_targets {
                    if target.kind == StdinTargetKind::SourceStandardInput {
                        continue;
                    }
                    // SAFETY: each target is a live recorded descriptor; shutdown
                    // only changes a socket's write direction and reports ENOTSOCK
                    // for the other descriptor kinds.
                    let shutdown =
                        unsafe { libc::shutdown(target.descriptor.as_raw_fd(), SHUT_WR) };
                    if shutdown != 0 {
                        // A pty has no half-close. In canonical mode EOT ends the
                        // slave's input without closing the master's output side.
                        let _ = write_all(target.descriptor.as_fd(), &[0x04]);
                    }
                }
                Ok(())
            }))
        };

        let mut first_error = None;
        if let Err(err) = receive_output(streams, &receiver) {
            first_error = Some(err);
        }
        for relay in relays {
            match relay.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) if err != nix::Error::EPIPE => {
                    first_error = first_error.or(Some(err));
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    first_error = first_error.or(Some(nix::Error::EIO));
                }
            }
        }
        if let Some(stdin_thread) = stdin_thread {
            match stdin_thread.join() {
                Ok(Ok(())) => {}
                Ok(Err(err)) if err != nix::Error::EPIPE => {
                    first_error = first_error.or(Some(err));
                }
                Ok(Err(_)) => {}
                Err(_) => {
                    first_error = first_error.or(Some(nix::Error::EIO));
                }
            }
        }
        first_error.map_or(Ok(()), Err)
    });
    result.map_err(|err| {
        if err != nix::Error::EPIPE {
            perror_nix("resume", err);
        }
        STATUS_CMD_ERROR
    })?;
    finish_outputs(streams)
}

/// Append every channel record to the command's output streams.
fn receive_output(
    streams: &mut IoStreams,
    receiver: &std::sync::mpsc::Receiver<(bool, Vec<u8>)>,
) -> Result<(), nix::Error> {
    for (is_stderr, data) in receiver {
        let target = if is_stderr {
            &mut *streams.err
        } else {
            &mut *streams.out
        };
        if !target.append(&bytes2wcstring(&data)) {
            return Err(nix::Error::EIO);
        }
    }
    Ok(())
}

/// Flush the command's output streams and report a write failure.
fn finish_outputs(streams: &mut IoStreams) -> BuiltinResult {
    let out = streams.out.flush_and_check_error();
    let err = streams.err.flush_and_check_error();
    if out == STATUS_CMD_OK && err == STATUS_CMD_OK {
        Ok(SUCCESS)
    } else {
        Err(STATUS_CMD_ERROR)
    }
}

/// Where a relayed stream is written.
#[derive(Clone, Copy)]
enum Sink {
    /// A descriptor of the shell.
    Fd(RawFd),
    /// A channel to the builtin, which appends the bytes to its output stream.
    Channel,
    /// The stream has no destination here.
    Closed,
}

/// Return the sink of the command's standard output.
fn stdout_sink(streams: &IoStreams) -> Sink {
    streams.out.fd().map_or(Sink::Channel, Sink::Fd)
}

/// Return the sink of the command's standard error.
fn stderr_sink(streams: &IoStreams) -> Sink {
    streams.err.fd().map_or(Sink::Channel, Sink::Fd)
}

/// Return the sink of the command's descriptor `descriptor`.
fn descriptor_sink(streams: &IoStreams, descriptor: RawFd) -> Sink {
    if descriptor == STDOUT_FILENO {
        stdout_sink(streams)
    } else if descriptor == STDERR_FILENO {
        stderr_sink(streams)
    } else {
        streams
            .io_chain
            .resolve_fd(descriptor)
            .map_or(Sink::Closed, Sink::Fd)
    }
}

/// Relay `stream` to `sink`, tagging channel records with `is_stderr`.
fn relay_to_sink(
    stream: Arc<OwnedFd>,
    sink: Sink,
    is_stderr: bool,
    sender: Sender<(bool, Vec<u8>)>,
) -> Result<(), nix::Error> {
    match sink {
        Sink::Fd(target_fd) => {
            // The target descriptor belongs to the shell and outlives this call.
            let target = unsafe { BorrowedFd::borrow_raw(target_fd) };
            relay(stream.as_fd(), target)
        }
        Sink::Channel => relay_to_channel(stream, is_stderr, &sender),
        Sink::Closed => Ok(()),
    }
}

/// Move everything `source` produces into `target`. A closed reader ends the relay.
fn relay(source: BorrowedFd<'_>, target: BorrowedFd<'_>) -> Result<(), nix::Error> {
    use nix::fcntl::{SpliceFFlags, splice};

    loop {
        match splice(
            source,
            None,
            target,
            None,
            128 * 1024,
            SpliceFFlags::empty(),
        ) {
            Ok(0) => return Ok(()),
            Ok(_) => continue,
            Err(nix::Error::EINTR) => continue,
            Err(nix::Error::EPIPE | nix::Error::EBADF) => return Ok(()),
            Err(nix::Error::EINVAL | nix::Error::ENOSYS | nix::Error::ENOTSUP) => {
                return copy(source, target);
            }
            Err(err) => return Err(err),
        }
    }
}

/// Copy everything `source` produces to `target`.
fn copy(source: BorrowedFd<'_>, target: BorrowedFd<'_>) -> Result<(), nix::Error> {
    let mut buffer = [0u8; 4096];
    loop {
        match read(source, &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => write_all(target, &buffer[..count])?,
            Err(nix::Error::EINTR) => continue,
            Err(nix::Error::EBADF) => return Ok(()),
            Err(err) => return Err(err),
        }
    }
}

/// Write `data` to `target`, treating a closed reader as the end of the relay.
fn write_all(target: BorrowedFd<'_>, mut data: &[u8]) -> Result<(), nix::Error> {
    while !data.is_empty() {
        match nix::unistd::write(target, data) {
            Ok(0) => return Ok(()),
            Ok(count) => data = &data[count..],
            Err(nix::Error::EINTR) => continue,
            Err(nix::Error::EPIPE | nix::Error::EBADF) => return Ok(()),
            Err(err) => return Err(err),
        }
    }
    Ok(())
}

/// Send everything `stream` produces to `sender` in chunks.
fn relay_to_channel(
    stream: Arc<OwnedFd>,
    is_stderr: bool,
    sender: &Sender<(bool, Vec<u8>)>,
) -> Result<(), nix::Error> {
    let mut buffer = [0u8; 4096];
    loop {
        match read(stream.as_fd(), &mut buffer) {
            Ok(0) => return Ok(()),
            Ok(count) => {
                if sender.send((is_stderr, buffer[..count].to_vec())).is_err() {
                    return Ok(());
                }
            }
            Err(nix::Error::EINTR) => continue,
            Err(nix::Error::EBADF) => return Ok(()),
            Err(err) => return Err(err),
        }
    }
}
