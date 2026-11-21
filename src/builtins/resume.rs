//! Implementation of the `resume` builtin.

use super::prelude::*;
use crate::io::OutputStream;
use crate::proc::SuspensionHandle;
use crate::wutil::perror_nix;
use fish_widestring::bytes2wcstring;
use libc::STDIN_FILENO;
use nix::unistd::read;
use std::os::fd::RawFd;

/// Consume a suspension and connect its stream to the command's standard output.
///
/// The suspension records the shell's end of the pipe whose other end a suspended command holds.
/// The pipe is linked to the command's output descriptor, so a pipeline stage after `resume`
/// receives the stream as any other stage's output.
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
    let Some(suspended_stdin) = suspension.take_descriptor(STDIN_FILENO) else {
        streams.err.appendln(&wgettext_fmt!(
            "resume: %s records no standard input stream",
            operands[0]
        ));
        return Err(STATUS_CMD_ERROR);
    };

    if streams.out.flush_and_check_error() != STATUS_CMD_OK {
        return Err(STATUS_CMD_ERROR);
    }
    if let Some(target_fd) = streams.out.fd() {
        if relay_stream_to_fd(&suspended_stdin, target_fd) {
            return Ok(SUCCESS);
        }
        // A descriptor which splice cannot serve is written from user space instead.
    }
    if copy_stream_to_output(&suspended_stdin, streams.out) {
        Ok(SUCCESS)
    } else {
        Err(STATUS_CMD_ERROR)
    }
}

/// Relay the bytes of `source` to `target_fd`, returning whether the relay completed.
///
/// On Linux the bytes move between the descriptors in the kernel. A target which the kernel cannot
/// splice to (a terminal, for example) is left to the caller's copy.
fn relay_stream_to_fd(source: &std::os::fd::OwnedFd, target_fd: RawFd) -> bool {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use nix::fcntl::{SpliceFFlags, splice};
        use std::os::fd::BorrowedFd;

        // The target descriptor belongs to the stream and outlives this call.
        let target = unsafe { BorrowedFd::borrow_raw(target_fd) };
        loop {
            match splice(
                source,
                None,
                target,
                None,
                128 * 1024,
                SpliceFFlags::empty(),
            ) {
                Ok(0) => return true,
                Ok(_) => continue,
                Err(nix::Error::EINTR) => continue,
                Err(nix::Error::EINVAL | nix::Error::ENOSYS | nix::Error::ENOTSUP) => {
                    return false;
                }
                Err(err) => {
                    perror_nix("splice", err);
                    return false;
                }
            }
        }
    }
    #[cfg(not(any(target_os = "linux", target_os = "android")))]
    {
        let _ = (source, target_fd);
        false
    }
}

/// Copy the bytes of `source` to `out`, returning whether the copy completed.
fn copy_stream_to_output(source: &std::os::fd::OwnedFd, out: &mut OutputStream) -> bool {
    let mut buffer = [0u8; 4096];
    loop {
        match read(source, &mut buffer) {
            Ok(0) => break,
            Ok(count) => {
                if !out.append(&bytes2wcstring(&buffer[..count])) {
                    return false;
                }
            }
            Err(nix::Error::EINTR) => continue,
            Err(err) => {
                perror_nix("read", err);
                return false;
            }
        }
    }
    out.flush_and_check_error() == STATUS_CMD_OK
}
