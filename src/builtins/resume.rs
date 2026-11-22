//! Implementation of the `resume` builtin.

use super::prelude::*;
use crate::proc::{Suspension, SuspensionDirection, SuspensionForm, SuspensionHandle};
use crate::wutil::perror_nix;
use fish_widestring::bytes2wcstring;
use libc::{STDERR_FILENO, STDIN_FILENO, STDOUT_FILENO};
use nix::unistd::{dup, read};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd, RawFd};
use std::sync::mpsc::{Sender, channel};
use std::thread::{self, ScopedJoinHandle};

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
    let Some(mut suspension) = parser.take_suspension(handle) else {
        streams.err.appendln(&wgettext_fmt!(
            "resume: %s denotes no suspension",
            operands[0]
        ));
        return Err(STATUS_CMD_ERROR);
    };

    if streams.out.flush_and_check_error() != STATUS_CMD_OK {
        return Err(STATUS_CMD_ERROR);
    }
    match suspension.form {
        SuspensionForm::Stream => resume_stream(streams, &mut suspension),
        SuspensionForm::Command => resume_command(streams, &mut suspension),
    }
}

/// Link a stream suspension's descriptor to the command's standard output.
fn resume_stream(streams: &mut IoStreams, suspension: &mut Suspension) -> BuiltinResult {
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
fn resume_command(streams: &mut IoStreams, suspension: &mut Suspension) -> BuiltinResult {
    let descriptors = suspension.take_all_descriptors();
    let (sender, receiver) = channel::<(bool, Vec<u8>)>();
    let result = thread::scope(|scope| {
        let mut relays: Vec<ScopedJoinHandle<'_, Result<(), nix::Error>>> = Vec::new();

        for (descriptor, (direction, stream)) in descriptors {
            match direction {
                SuspensionDirection::In => {
                    if descriptor == STDIN_FILENO {
                        if streams.stdin_is_directly_redirected {
                            let Some(stdin_file) = streams.stdin_file.as_ref() else {
                                continue;
                            };
                            match dup(stdin_file.as_fd()) {
                                Ok(source) => {
                                    relays.push(
                                        scope.spawn(move || relay(source.as_fd(), stream.as_fd())),
                                    );
                                }
                                Err(err) => {
                                    perror_nix("dup", err);
                                    return Err(err);
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
            }
        }
        drop(sender);

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
    stream: OwnedFd,
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
    stream: OwnedFd,
    is_stderr: bool,
    sender: &Sender<(bool, Vec<u8>)>,
) -> Result<(), nix::Error> {
    let mut buffer = [0u8; 4096];
    loop {
        match read(&stream, &mut buffer) {
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
