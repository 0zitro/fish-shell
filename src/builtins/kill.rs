//! Implementation of the `kill` builtin.
//!
//! The command form follows util-linux `kill(1)`: signals are named or numbered,
//! `-l` and `-L` inspect the signal namespace, and every target is signalled
//! independently. Fish job specifications are accepted in addition to the
//! process identifiers the system `kill(2)` accepts.

use super::prelude::*;
use crate::err_fmt;
#[cfg(target_os = "linux")]
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
#[cfg(target_os = "linux")]
use std::os::unix::fs::MetadataExt as _;

/// The status util-linux returns when some targets were signalled and some failed.
const KILL_EXIT_SOMEOK: c_int = 64;

localizable_consts!(
    KILL_PENDING_THREAD "Pending (thread)"
    KILL_PENDING_PROCESS "Pending (process)"
    KILL_BLOCKED "Blocked"
    KILL_IGNORED "Ignored"
    KILL_CAUGHT "Caught"
);

/// One entry of the signal namespace printed by `-l` and `-L`.
struct SignalName {
    number: i32,
    name: &'static wstr,
}

macro_rules! signal_name {
    ($number:expr, $name:literal) => {
        SignalName {
            number: $number,
            name: L!($name),
        }
    };
}

/// The signal names in the order util-linux prints them.
#[rustfmt::skip]
const SIGNAL_NAMES: &[SignalName] = &[
    signal_name!(libc::SIGHUP, "HUP"),
    signal_name!(libc::SIGINT, "INT"),
    signal_name!(libc::SIGQUIT, "QUIT"),
    signal_name!(libc::SIGILL, "ILL"),
    signal_name!(libc::SIGTRAP, "TRAP"),
    signal_name!(libc::SIGABRT, "ABRT"),
    #[cfg(target_os = "linux")]
    signal_name!(libc::SIGIOT, "IOT"),
    signal_name!(libc::SIGBUS, "BUS"),
    signal_name!(libc::SIGFPE, "FPE"),
    signal_name!(libc::SIGKILL, "KILL"),
    signal_name!(libc::SIGUSR1, "USR1"),
    signal_name!(libc::SIGSEGV, "SEGV"),
    signal_name!(libc::SIGUSR2, "USR2"),
    signal_name!(libc::SIGPIPE, "PIPE"),
    signal_name!(libc::SIGALRM, "ALRM"),
    signal_name!(libc::SIGTERM, "TERM"),
    #[cfg(target_os = "linux")]
    signal_name!(libc::SIGSTKFLT, "STKFLT"),
    signal_name!(libc::SIGCHLD, "CHLD"),
    #[cfg(target_os = "linux")]
    signal_name!(libc::SIGCHLD, "CLD"),
    signal_name!(libc::SIGCONT, "CONT"),
    signal_name!(libc::SIGSTOP, "STOP"),
    signal_name!(libc::SIGTSTP, "TSTP"),
    signal_name!(libc::SIGTTIN, "TTIN"),
    signal_name!(libc::SIGTTOU, "TTOU"),
    signal_name!(libc::SIGURG, "URG"),
    signal_name!(libc::SIGXCPU, "XCPU"),
    signal_name!(libc::SIGXFSZ, "XFSZ"),
    signal_name!(libc::SIGVTALRM, "VTALRM"),
    signal_name!(libc::SIGPROF, "PROF"),
    signal_name!(libc::SIGWINCH, "WINCH"),
    signal_name!(libc::SIGIO, "IO"),
    #[cfg(target_os = "linux")]
    signal_name!(libc::SIGPOLL, "POLL"),
    #[cfg(target_os = "linux")]
    signal_name!(libc::SIGPWR, "PWR"),
    signal_name!(libc::SIGSYS, "SYS"),
];

/// One signal number.
///
/// `SignalNumber` admits zero, which is a valid null signal for `kill(2)` but
/// not a signal disposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct SignalNumber(i32);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum KillTarget {
    Pid(i32),
    PidFd { pid: i32, inode: u64 },
}

impl KillTarget {
    fn pid(self) -> i32 {
        match self {
            KillTarget::Pid(pid) | KillTarget::PidFd { pid, .. } => pid,
        }
    }
}

struct TimeoutFollowUp {
    milliseconds: i32,
    signal: i32,
}

impl SignalNumber {
    fn new(number: i32) -> Self {
        SignalNumber(number)
    }

    fn code(self) -> i32 {
        self.0
    }
}

/// The first number after the last conventional signal.
fn signal_number_limit() -> i32 {
    #[cfg(target_os = "linux")]
    {
        libc::SIGRTMAX() + 1
    }
    #[cfg(not(target_os = "linux"))]
    {
        128
    }
}

fn signal_name_for(number: i32) -> WString {
    if number == 0 {
        return L!("0").to_owned();
    }
    for entry in SIGNAL_NAMES {
        if entry.number == number {
            return entry.name.to_owned();
        }
    }
    #[cfg(target_os = "linux")]
    {
        let realtime_min = libc::SIGRTMIN();
        let realtime_max = libc::SIGRTMAX();
        if (realtime_min..=realtime_max).contains(&number) {
            return sprintf!("RT%d", number - realtime_min);
        }
    }
    sprintf!("%d", number)
}

fn signal_number_from_name(name: &wstr) -> Option<SignalNumber> {
    let mut name = name;
    if name.char_count() >= 3 && name.slice_to(3).to_string().eq_ignore_ascii_case("sig") {
        name = name.slice_from(3);
    }

    #[cfg(target_os = "linux")]
    {
        if name.char_count() >= 2 && name.slice_to(2).to_string().eq_ignore_ascii_case("rt") {
            let rest = name.slice_from(2);
            let (offset, from_max) = if let Some(digits) = rest.strip_prefix(L!("min+")) {
                (parse_signal_offset(digits)?, false)
            } else if let Some(digits) = rest.strip_prefix(L!("max-")) {
                (parse_signal_offset(digits)?, true)
            } else {
                (parse_signal_offset(rest)?, false)
            };
            let number = if from_max {
                libc::SIGRTMAX() - offset
            } else {
                libc::SIGRTMIN() + offset
            };
            if (libc::SIGRTMIN()..=libc::SIGRTMAX()).contains(&number) {
                return Some(SignalNumber::new(number));
            }
            return None;
        }
    }

    for entry in SIGNAL_NAMES {
        if entry
            .name
            .to_string()
            .eq_ignore_ascii_case(&name.to_string())
        {
            return Some(SignalNumber::new(entry.number));
        }
    }
    None
}

fn parse_signal_offset(digits: &wstr) -> Option<i32> {
    if digits.is_empty() || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    fish_wcstoi(digits).ok()
}

/// Parse a signal specification used by `-s` and by a bare `-SIGNAL`.
fn parse_signal_argument(argument: &wstr, mask_bit: bool) -> Option<SignalNumber> {
    if argument.is_empty() {
        return None;
    }
    if argument.char_at(0).is_ascii_digit() {
        let number = fish_wcstoi(argument).ok()?;
        let number = if mask_bit && number >= signal_number_limit() && number & 128 != 0 {
            number - 128
        } else {
            number
        };
        if !(0..signal_number_limit()).contains(&number) {
            return None;
        }
        return Some(SignalNumber::new(number));
    }
    signal_number_from_name(argument)
}

fn parse_signal_mask(argument: &wstr) -> Option<u64> {
    let digits = argument.strip_prefix(L!("0x"))?;
    u64::from_str_radix(&digits.to_string(), 16).ok()
}

fn signal_name_table_row(number: i32, name: &wstr) -> WString {
    sprintf!("%2d %-8s", number, name)
}

fn print_signal_name(streams: &mut IoStreams, signal: SignalNumber, newline: bool) {
    let name = signal_name_for(signal.code());
    streams.out.append(&name);
    if newline {
        streams.out.append(L!("\n"));
    }
}

fn print_signal_mask(streams: &mut IoStreams, mask: u64, separator: char) {
    for bit in 0..u64::BITS {
        if mask & (1 << bit) != 0 {
            streams.out.append(&signal_name_for((bit + 1) as i32));
            streams.out.append(&separator.to_string());
        }
    }
}

fn print_signal_mask_lines(streams: &mut IoStreams, mask: u64) {
    for bit in 0..u64::BITS {
        if mask & (1 << bit) != 0 {
            print_signal_name(streams, SignalNumber::new((bit + 1) as i32), true);
        }
    }
}

#[cfg(target_os = "linux")]
fn process_name(pid: i32) -> Option<String> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let mut start = stat.find('(')? + 1;
    while stat[start..].starts_with('(') {
        start += 1;
    }
    let end = stat[start..].find(')')? + start;
    Some(stat[start..end].to_owned())
}

#[cfg(target_os = "linux")]
fn process_sigcatch(pid: i32) -> Option<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat")).ok()?;
    let end = stat.rfind(')')? + 1;
    let fields: Vec<&str> = stat[end..].split_whitespace().collect();
    u64::from_str_radix(fields.get(31)?, 16).ok()
}

#[cfg(target_os = "linux")]
fn matching_process_ids(name: &wstr, all_users: bool) -> Vec<i32> {
    let name = name.to_string();
    // SAFETY: `getuid` has no preconditions and touches no Rust value.
    let uid = unsafe { libc::getuid() };
    let Ok(entries) = std::fs::read_dir("/proc") else {
        return Vec::new();
    };

    let mut result = Vec::new();
    for entry in entries.flatten() {
        let Ok(pid) = entry.file_name().to_string_lossy().parse::<i32>() else {
            continue;
        };
        if !all_users {
            let Ok(metadata) = entry.metadata() else {
                continue;
            };
            if metadata.uid() != uid {
                continue;
            }
        }
        if process_name(pid).is_some_and(|candidate| candidate == name) {
            result.push(pid);
        }
    }
    result
}

#[cfg(not(target_os = "linux"))]
fn matching_process_ids(_name: &wstr, _all_users: bool) -> Vec<i32> {
    Vec::new()
}

#[cfg(target_os = "linux")]
fn signal_has_handler(pid: i32, signal: SignalNumber) -> bool {
    if signal.code() == 0 {
        return true;
    }
    let Some(sigcatch) = process_sigcatch(pid) else {
        return false;
    };
    let bit = u32::try_from(signal.code() - 1).ok();
    bit.is_some_and(|bit| sigcatch & (1u64 << bit) != 0)
}

#[cfg(not(target_os = "linux"))]
fn signal_has_handler(_pid: i32, _signal: SignalNumber) -> bool {
    true
}

fn print_process_signal_state(streams: &mut IoStreams, pid: i32) -> BuiltinResult {
    #[cfg(target_os = "linux")]
    {
        let path = format!("/proc/{pid}/status");
        let Ok(status) = std::fs::read_to_string(&path) else {
            err_fmt!("cannot open %s", path).finish(streams);
            return Err(STATUS_CMD_ERROR);
        };

        let fields = [
            ("SigPnd:", KILL_PENDING_THREAD),
            ("ShdPnd:", KILL_PENDING_PROCESS),
            ("SigBlk:", KILL_BLOCKED),
            ("SigIgn:", KILL_IGNORED),
            ("SigCgt:", KILL_CAUGHT),
        ];
        for line in status.lines() {
            for (key, label) in &fields {
                if let Some(value) = line.strip_prefix(key) {
                    let value = value.trim();
                    let Ok(mask) = u64::from_str_radix(value, 16) else {
                        err_fmt!("unexpected sigmask format: %s", value).finish(streams);
                        continue;
                    };
                    if mask != 0 {
                        streams.out.append(wgettext!(label));
                        streams.out.append(L!(": "));
                        print_signal_mask(streams, mask, ' ');
                        streams.out.append(L!("\n"));
                    }
                }
            }
        }
        Ok(SUCCESS)
    }
    #[cfg(not(target_os = "linux"))]
    {
        let _ = (streams, pid);
        Err(STATUS_CMD_ERROR)
    }
}

#[cfg(target_os = "linux")]
fn open_pidfd(pid: i32, inode: u64) -> Result<OwnedFd, nix::Error> {
    // SAFETY: `pidfd_open` takes a PID and flags; no Rust value is involved.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, pid, 0) };
    if fd < 0 {
        return Err(nix::Error::last());
    }
    // SAFETY: `pidfd_open` returned a fresh descriptor owned by this call.
    let fd = unsafe { OwnedFd::from_raw_fd(fd as i32) };

    if inode != 0 {
        // SAFETY: the descriptor is live and `statbuf` is a zeroed local.
        let mut statbuf: libc::stat = unsafe { std::mem::zeroed() };
        // SAFETY: the descriptor is live and the pointer is a valid `stat`.
        if unsafe { libc::fstat(fd.as_raw_fd(), &mut statbuf) } < 0 {
            return Err(nix::Error::last());
        }
        if statbuf.st_ino as u64 != inode {
            return Err(nix::Error::ENOENT);
        }
    }
    Ok(fd)
}

#[cfg(target_os = "linux")]
#[repr(C)]
struct QueuedSignalInfo {
    signal: i32,
    error: i32,
    code: i32,
    _padding: i32,
    pid: i32,
    uid: u32,
    value: libc::sigval,
    _rest: [u8; std::mem::size_of::<libc::siginfo_t>() - 32],
}

#[cfg(target_os = "linux")]
const _: () =
    assert!(std::mem::size_of::<QueuedSignalInfo>() == std::mem::size_of::<libc::siginfo_t>());

#[cfg(target_os = "linux")]
impl QueuedSignalInfo {
    fn new(signal: i32, value: i32) -> Self {
        QueuedSignalInfo {
            signal,
            error: 0,
            code: libc::SI_QUEUE,
            _padding: 0,
            // SAFETY: `getpid` and `getuid` have no preconditions and touch
            // no Rust value.
            pid: unsafe { libc::getpid() },
            uid: unsafe { libc::getuid() },
            value: libc::sigval {
                sival_ptr: (value as isize) as *mut libc::c_void,
            },
            _rest: [0; std::mem::size_of::<libc::siginfo_t>() - 32],
        }
    }
}

#[cfg(target_os = "linux")]
fn send_via_pidfd(fd: &OwnedFd, signal: i32) -> Result<(), nix::Error> {
    // SAFETY: the descriptor is a live pidfd and passing no siginfo is
    // permitted by `pidfd_send_signal`.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if result < 0 {
        return Err(nix::Error::last());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn send_via_pidfd_with_info(
    fd: &OwnedFd,
    signal: i32,
    info: &mut QueuedSignalInfo,
) -> Result<(), nix::Error> {
    info.signal = signal;
    // SAFETY: `QueuedSignalInfo` has the ABI size of `siginfo_t`, is fully
    // initialised, and the syscall only reads it for the duration of the call.
    let result = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            signal,
            info as *mut QueuedSignalInfo as *mut libc::siginfo_t,
            0,
        )
    };
    if result < 0 {
        return Err(nix::Error::last());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn send_with_timeout(
    pid: i32,
    inode: u64,
    signal: i32,
    value: Option<i32>,
    follow_ups: &[TimeoutFollowUp],
    verbose: bool,
    streams: &mut IoStreams,
) -> Result<(), nix::Error> {
    let fd = open_pidfd(pid, inode)?;
    let mut info = QueuedSignalInfo::new(signal, value.unwrap_or(signal));
    send_via_pidfd_with_info(&fd, signal, &mut info)?;

    for follow_up in follow_ups {
        let mut pollfd = libc::pollfd {
            fd: fd.as_raw_fd(),
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: `poll` reads one local `pollfd` and writes its `revents`.
        let ready = unsafe { libc::poll(&mut pollfd, 1, follow_up.milliseconds) };
        if ready < 0 {
            return Err(nix::Error::last());
        }
        if ready == 0 {
            if verbose {
                streams.out.appendln(&wgettext_fmt!(
                    "timeout, sending signal %d to pid %d",
                    follow_up.signal,
                    pid
                ));
            }
            send_via_pidfd_with_info(&fd, follow_up.signal, &mut info)?;
        }
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn send_with_timeout(
    _pid: i32,
    _inode: u64,
    _signal: i32,
    _value: Option<i32>,
    _follow_ups: &[TimeoutFollowUp],
    _verbose: bool,
    _streams: &mut IoStreams,
) -> Result<(), nix::Error> {
    Err(nix::Error::ENOSYS)
}

fn print_all_signal_names(streams: &mut IoStreams, with_numbers: bool) {
    let pretty = streams.out_is_terminal();

    if !with_numbers {
        if !pretty {
            for entry in SIGNAL_NAMES {
                streams.out.append(entry.name);
                streams.out.append(L!("\n"));
            }
            #[cfg(target_os = "linux")]
            {
                streams.out.appendln(L!("RT<N>"));
                streams.out.appendln(L!("RTMIN+<N>"));
                streams.out.appendln(L!("RTMAX-<N>"));
            }
            return;
        }

        let mut line = WString::new();
        for entry in SIGNAL_NAMES {
            let item = sprintf!(" %s", entry.name);
            if line.char_count() + item.char_count() > 72 {
                streams.out.appendln(&line);
                line.clear();
            }
            line.push_utfstr(&item);
        }
        #[cfg(target_os = "linux")]
        {
            let suffix = L!(" RT<N> RTMIN+<N> RTMAX-<N>");
            if line.char_count() + suffix.char_count() > 72 {
                streams.out.appendln(&line);
                line.clear();
            }
            line.push_utfstr(suffix);
        }
        if !line.is_empty() {
            streams.out.appendln(&line);
        }
        return;
    }

    if !pretty {
        for entry in SIGNAL_NAMES {
            streams
                .out
                .appendln(&signal_name_table_row(entry.number, entry.name));
        }
        #[cfg(target_os = "linux")]
        {
            streams
                .out
                .appendln(&signal_name_table_row(libc::SIGRTMIN(), L!("RTMIN")));
            streams
                .out
                .appendln(&signal_name_table_row(libc::SIGRTMAX(), L!("RTMAX")));
        }
        return;
    }

    let mut line = WString::new();
    let mut columns = 0;
    for entry in SIGNAL_NAMES {
        if columns == 6 {
            streams.out.appendln(&line);
            line.clear();
            columns = 0;
        }
        line.push_utfstr(&signal_name_table_row(entry.number, entry.name));
        columns += 1;
    }
    #[cfg(target_os = "linux")]
    {
        if columns == 6 {
            streams.out.appendln(&line);
            line.clear();
        }
        line.push_utfstr(&signal_name_table_row(libc::SIGRTMIN(), L!("RTMIN")));
        line.push_utfstr(&signal_name_table_row(libc::SIGRTMAX(), L!("RTMAX")));
    }
    if !line.is_empty() {
        streams.out.appendln(&line);
    }
}

fn print_signal_list(streams: &mut IoStreams, argument: Option<&wstr>) -> BuiltinResult {
    let Some(argument) = argument else {
        print_all_signal_names(streams, false);
        return Ok(SUCCESS);
    };

    if argument.starts_with(L!("0x")) {
        let Some(mask) = parse_signal_mask(argument) else {
            err_fmt!("invalid sigmask format: %s", argument).finish(streams);
            return Err(STATUS_CMD_ERROR);
        };
        print_signal_mask_lines(streams, mask);
        return Ok(SUCCESS);
    }

    let Some(signal) = parse_signal_argument(argument, true) else {
        err_fmt!("unknown signal: %s", argument).finish(streams);
        return Err(STATUS_CMD_ERROR);
    };
    print_signal_name(streams, signal, true);
    Ok(SUCCESS)
}

fn print_signal_table(streams: &mut IoStreams) -> BuiltinResult {
    print_all_signal_names(streams, true);
    Ok(SUCCESS)
}

fn print_kill_version(streams: &mut IoStreams) -> BuiltinResult {
    #[cfg(target_os = "linux")]
    let features = L!(" (with: sigqueue, pidfd, pidfdino)");
    #[cfg(not(target_os = "linux"))]
    let features = L!("");

    streams.out.appendln(&wgettext_fmt!(
        "kill from fish %s%s",
        crate::BUILD_VERSION,
        features
    ));
    Ok(SUCCESS)
}

#[derive(Default)]
struct KillRun {
    attempted: usize,
    failed: usize,
}

impl KillRun {
    fn status(&self) -> BuiltinResult {
        if self.attempted != 0 && self.failed == 0 {
            Ok(SUCCESS)
        } else if self.attempted == self.failed {
            Err(STATUS_CMD_ERROR)
        } else {
            Err(KILL_EXIT_SOMEOK)
        }
    }
}

/// Parse one numeric PID or `pid:pidfd_inode` target.
fn parse_target(argument: &wstr) -> Option<KillTarget> {
    if let Some(colon) = argument.find_char(':') {
        let pid = fish_wcstoi(argument.slice_to(colon)).ok()?;
        let inode = argument
            .slice_from(colon + 1)
            .to_string()
            .parse::<u64>()
            .ok()?;
        if pid <= 0 || inode == 0 {
            return None;
        }
        return Some(KillTarget::PidFd { pid, inode });
    }
    fish_wcstoi(argument).ok().map(KillTarget::Pid)
}

fn signal_argument_and_targets(
    parser: &mut Parser,
    streams: &mut IoStreams,
    argv: &[&wstr],
) -> BuiltinResult {
    let command = argv[0];
    let mut signal = SignalNumber::new(libc::SIGTERM);
    let mut do_kill = false;
    let mut do_pid = false;
    let mut all_users = false;
    let mut require_handler = false;
    let mut verbose = false;
    let mut queue_value = None;
    let mut follow_ups: Vec<TimeoutFollowUp> = Vec::new();
    let mut index = 1;

    while index < argv.len() {
        let argument = argv[index];
        if argument == "--" {
            index += 1;
            break;
        }
        if argument == "-h" || argument == "--help" {
            builtin_print_help(parser, streams, command);
            return Ok(SUCCESS);
        }
        if argument == "-V" || argument == "-v" || argument == "--version" {
            return print_kill_version(streams);
        }
        if argument == "-l" || argument == "--list" {
            let argument = argv.get(index + 1).copied();
            if argument.is_some() && index + 2 < argv.len() {
                err_fmt!("too many arguments").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            return print_signal_list(streams, argument);
        }
        if let Some(argument) = argument
            .strip_prefix(L!("--list="))
            .or_else(|| argument.strip_prefix(L!("-l=")))
        {
            return print_signal_list(streams, Some(argument));
        }
        if argument == "-L" || argument == "--table" {
            return print_signal_table(streams);
        }
        if argument == "-a" || argument == "--all" {
            all_users = true;
            index += 1;
            continue;
        }
        if argument == "-r" || argument == "--require-handler" {
            require_handler = true;
            index += 1;
            continue;
        }
        if argument == "--verbose" {
            verbose = true;
            index += 1;
            continue;
        }
        if argument == "-p" || argument == "--pid" {
            if do_kill {
                err_fmt!("--pid and --signal are mutually exclusive").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            if queue_value.is_some() {
                err_fmt!("--pid and --queue are mutually exclusive").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            do_pid = true;
            index += 1;
            continue;
        }
        if argument == "-q" || argument == "--queue" {
            if do_pid {
                err_fmt!("--pid and --queue are mutually exclusive").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            let Some(value) = argv.get(index + 1) else {
                err_fmt!("option '%s' requires an argument", argument).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            let Ok(value) = fish_wcstoi(value) else {
                err_fmt!("argument error").finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            queue_value = Some(value);
            index += 2;
            continue;
        }
        if argument == "--timeout" {
            let Some(milliseconds) = argv.get(index + 1) else {
                err_fmt!("option '%s' requires an argument", argument).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            let Some(signal_argument) = argv.get(index + 2) else {
                err_fmt!("option '%s' requires an argument", argument).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            let Ok(milliseconds) = fish_wcstoi(milliseconds) else {
                err_fmt!("argument error").finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            let Some(follow_up_signal) = parse_signal_argument(signal_argument, false) else {
                err_fmt!("unknown signal: %s", signal_argument).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            follow_ups.push(TimeoutFollowUp {
                milliseconds,
                signal: follow_up_signal.code(),
            });
            index += 3;
            continue;
        }
        if argument == "-d" || argument == "--show-process-state" {
            if index + 1 >= argv.len() {
                err_fmt!("too few arguments").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            if index + 2 < argv.len() {
                err_fmt!("too many arguments").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            let Some(pid) = parse_positive_pid(argv[index + 1]) else {
                err_fmt!("invalid PID argument").finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            return print_process_signal_state(streams, pid);
        }
        if let Some(pid) = argument
            .strip_prefix(L!("--show-process-state="))
            .or_else(|| argument.strip_prefix(L!("-d=")))
        {
            let Some(pid) = parse_positive_pid(pid) else {
                err_fmt!("invalid PID argument").finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            return print_process_signal_state(streams, pid);
        }
        if argument == "-s" || argument == "--signal" {
            if do_pid {
                err_fmt!("--pid and --signal are mutually exclusive").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            let Some(value) = argv.get(index + 1) else {
                err_fmt!("not enough arguments").finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            let Some(parsed) = parse_signal_argument(value, false) else {
                err_fmt!("unknown signal: %s", value).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            signal = parsed;
            do_kill = true;
            index += 2;
            continue;
        }
        if argument.char_at(0) == '-' {
            if do_kill {
                break;
            }
            if do_pid {
                err_fmt!("--pid and --signal are mutually exclusive").finish(streams);
                return Err(STATUS_CMD_ERROR);
            }
            let candidate = argument.slice_from(1);
            let Some(parsed) = parse_signal_argument(candidate, false) else {
                err_fmt!("invalid signal name or number: %s", candidate).finish(streams);
                return Err(STATUS_CMD_ERROR);
            };
            signal = parsed;
            do_kill = true;
            index += 1;
            continue;
        }
        break;
    }

    if index >= argv.len() {
        err_fmt!("not enough arguments").finish(streams);
        return Err(STATUS_CMD_ERROR);
    }
    if !do_kill {
        signal = SignalNumber::new(libc::SIGTERM);
    }

    let mut run = KillRun::default();
    for argument in &argv[index..] {
        let targets = match parse_target(argument) {
            Some(target) => vec![target],
            None => matching_process_ids(argument, all_users)
                .into_iter()
                .map(KillTarget::Pid)
                .collect(),
        };
        if targets.is_empty() {
            err_fmt!("cannot find process \"%s\"", argument).finish(streams);
            run.attempted += 1;
            run.failed += 1;
            continue;
        }

        for target in targets {
            let pid = target.pid();
            let handler_present = !require_handler || signal_has_handler(pid, signal);
            if !handler_present {
                if verbose {
                    streams.out.appendln(&wgettext_fmt!(
                        "not signalling pid %d, it has no userspace handler for signal %d",
                        pid,
                        signal.code()
                    ));
                }
                continue;
            }

            run.attempted += 1;
            if verbose {
                streams.out.appendln(&wgettext_fmt!(
                    "sending signal %d to pid %d",
                    signal.code(),
                    pid
                ));
            }

            if do_pid {
                streams.out.appendln(&sprintf!("%d", pid));
                continue;
            }

            let result = if !follow_ups.is_empty() {
                let inode = match target {
                    KillTarget::Pid(_) => 0,
                    KillTarget::PidFd { inode, .. } => inode,
                };
                send_with_timeout(
                    pid,
                    inode,
                    signal.code(),
                    queue_value,
                    &follow_ups,
                    verbose,
                    streams,
                )
            } else if let Some(value) = queue_value {
                send_with_queue(pid, signal.code(), value)
            } else {
                match target {
                    KillTarget::Pid(_) => send_with_kill(pid, signal.code()),
                    KillTarget::PidFd { inode, .. } => {
                        open_pidfd(pid, inode).and_then(|fd| send_via_pidfd(&fd, signal.code()))
                    }
                }
            };

            if let Err(err) = result {
                run.failed += 1;
                err_fmt!("sending signal to %s failed: %s", argument, err.to_string())
                    .finish(streams);
            }
        }
    }
    run.status()
}

fn send_with_kill(pid: i32, signal: i32) -> Result<(), nix::Error> {
    // SAFETY: `kill(2)` only reads the two scalar arguments and the process
    // table; it holds no Rust value and assumes no pointer or lifetime.
    let result = unsafe { libc::kill(pid, signal) };
    if result < 0 {
        return Err(nix::Error::last());
    }
    Ok(())
}

#[cfg(target_os = "linux")]
fn send_with_queue(pid: i32, signal: i32, value: i32) -> Result<(), nix::Error> {
    let value = libc::sigval {
        sival_ptr: (value as isize) as *mut libc::c_void,
    };
    // SAFETY: `sigqueue` takes a PID, a signal and a scalar `sigval`; it holds
    // no Rust reference and assumes no pointer or lifetime.
    let result = unsafe { libc::sigqueue(pid, signal, value) };
    if result < 0 {
        return Err(nix::Error::last());
    }
    Ok(())
}

#[cfg(not(target_os = "linux"))]
fn send_with_queue(_pid: i32, _signal: i32, _value: i32) -> Result<(), nix::Error> {
    Err(nix::Error::ENOSYS)
}

fn parse_positive_pid(argument: &wstr) -> Option<i32> {
    let pid = fish_wcstoi(argument).ok()?;
    (pid > 0).then_some(pid)
}

/// The `kill` builtin.
pub fn kill(parser: &mut Parser, streams: &mut IoStreams, argv: &mut [&wstr]) -> BuiltinResult {
    if argv.is_empty() {
        return Err(STATUS_INVALID_ARGS);
    }
    signal_argument_and_targets(parser, streams, argv)
}

#[cfg(test)]
mod safety_tests {
    use super::*;

    #[cfg(target_os = "linux")]
    #[test]
    fn queued_signal_info_matches_the_siginfo_layout() {
        assert_eq!(
            std::mem::size_of::<QueuedSignalInfo>(),
            std::mem::size_of::<libc::siginfo_t>()
        );
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, signal), 0);
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, error), 4);
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, code), 8);
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, pid), 16);
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, uid), 20);
        assert_eq!(std::mem::offset_of!(QueuedSignalInfo, value), 24);
    }

    #[test]
    fn signal_numbers_keep_zero_distinct_from_signals() {
        assert_eq!(parse_signal_argument(L!("0"), false), Some(SignalNumber(0)));
        assert_eq!(
            parse_signal_argument(L!("TERM"), false),
            Some(SignalNumber(libc::SIGTERM))
        );
        assert_eq!(parse_signal_argument(L!("NOSUCHSIGNAL"), false), None);
    }
}

#[allow(unexpected_cfgs)]
#[cfg(kani)]
mod kani_harnesses {
    use super::*;

    #[cfg(target_os = "linux")]
    #[kani::proof]
    fn queued_signal_info_layout() {
        kani::assert(
            std::mem::size_of::<QueuedSignalInfo>() == std::mem::size_of::<libc::siginfo_t>(),
            "QueuedSignalInfo has the siginfo_t size",
        );
        kani::assert(
            std::mem::offset_of!(QueuedSignalInfo, signal) == 0,
            "signal starts the siginfo_t layout",
        );
        kani::assert(
            std::mem::offset_of!(QueuedSignalInfo, pid) == 16,
            "pid follows the fixed siginfo_t header",
        );
        kani::assert(
            std::mem::offset_of!(QueuedSignalInfo, uid) == 20,
            "uid follows pid",
        );
        kani::assert(
            std::mem::offset_of!(QueuedSignalInfo, value) == 24,
            "sigval follows the queued-signal fields",
        );
    }
}
