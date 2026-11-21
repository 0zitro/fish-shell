//! Implementation of the `suspend` builtin.

use super::prelude::*;
use crate::proc::{Suspension, get_login, is_interactive_session};
use crate::wutil::perror_nix;
use libc::STDIN_FILENO;
use nix::sys::signal::{Signal, kill};
use nix::unistd::{dup, getpid};
use std::os::fd::AsFd as _;

/// Suspend the current shell, or record the stream its input carries.
///
/// The shell stops itself with `SIGSTOP` rather than `SIGTSTP`: an interactive shell ignores
/// `SIGTSTP`, and the stop is what returns control to the controlling process. `SIGCONT` resumes
/// the shell.
///
/// When standard input is directly redirected, the command records the shell's own copy of the
/// input descriptor as a suspension and writes the handle that denotes it. The input is the stream
/// a pipe delivers, whatever stage follows.
pub fn suspend(parser: &mut Parser, streams: &mut IoStreams, argv: &mut [&wstr]) -> BuiltinResult {
    let cmd = argv[0];

    let shortopts: &wstr = L!("fh");
    let longopts: &[WOption] = &[
        wopt(L!("force"), ArgType::NoArgument, 'f'),
        wopt(L!("help"), ArgType::NoArgument, 'h'),
    ];

    let mut force = false;
    let mut print_help = false;
    let mut w = WGetopter::new(shortopts, longopts, argv);
    while let Some(c) = w.next_opt() {
        match c {
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
        let mut suspension = Suspension::new();
        suspension.record_descriptor(STDIN_FILENO, shell_end);
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
