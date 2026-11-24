use super::prelude::*;
use crate::err_fmt;
use crate::job_group::{JobId, MaybeJobId};
use crate::proc::{InternalJobId, Job, Pid, proc_clean_jobs, proc_wait_any};
use crate::signal::SigChecker;
use crate::wait_handle::{WaitHandleRef, WaitHandleStore};
use std::num::NonZeroU32;
use std::rc::Rc;

/// Return true if we can wait on a job.
fn can_wait_on_job(j: &Job) -> bool {
    j.is_constructed() && !j.is_foreground() && !j.is_stopped()
}

/// Return true if a wait handle matches a pid or a process name.
fn wait_handle_matches(query: WaitHandleQuery, wh: &WaitHandleRef) -> bool {
    match query {
        WaitHandleQuery::Pid(pid) => wh.pid == pid,
        WaitHandleQuery::ProcName(proc_name) => proc_name == wh.base_name,
    }
}

/// Return true if all chars are numeric.
fn iswnumeric(s: &wstr) -> bool {
    s.chars().all(|c| c.is_ascii_digit())
}

#[derive(Copy, Clone)]
enum WaitHandleQuery<'a> {
    Pid(Pid),
    ProcName(&'a wstr),
}

/// Walk the list of jobs, looking for a process with the given pid or proc name.
/// Append all matching wait handles to `handles`.
/// Return true if we found a matching job (even if not waitable), false if not.
fn find_wait_handles(
    query: WaitHandleQuery<'_>,
    parser: &mut Parser,
    handles: &mut Vec<WaitHandleRef>,
) -> bool {
    // Has a job already completed?
    let mut matched = false;
    let wait_handles: &mut WaitHandleStore = parser.mut_wait_handles();
    match query {
        WaitHandleQuery::Pid(pid) => {
            if let Some(wh) = wait_handles.get_by_pid(pid) {
                handles.push(wh);
                matched = true;
            }
        }
        _ => {
            for wh in wait_handles.iter() {
                if wait_handle_matches(query, wh) {
                    handles.push(wh.clone());
                    matched = true;
                }
            }
        }
    }

    // Is there a running job match?
    for j in parser.jobs() {
        if parser.job_contains_calling_task(j) {
            continue;
        }
        // We want to set 'matched' to true if we could have matched, even if the job was stopped.
        let provide_handle = can_wait_on_job(j);
        let internal_job_id = j.internal_job_id;
        for proc in j.processes().iter() {
            let Some(wh) = proc.make_wait_handle(internal_job_id) else {
                continue;
            };
            if wait_handle_matches(query, &wh) {
                matched = true;
                if provide_handle {
                    handles.push(wh);
                }
            }
        }
    }
    matched
}

/// One thing a wait observes.
enum WaitTarget {
    /// A job completes.
    Job(Rc<Job>),
    /// A child process completes.
    Handle(WaitHandleRef),
}

impl WaitTarget {
    /// Return the identity the target writes with `--emit`.
    fn identity(&self) -> WString {
        match self {
            WaitTarget::Job(job) => {
                let mut identity = WString::from(L!("%"));
                identity.push_utfstr(&job.job_id().as_num().to_wstring());
                identity
            }
            WaitTarget::Handle(handle) => handle.pid.to_wstring(),
        }
    }

    /// Return the target's status when it has completed.
    fn status(&self) -> Option<i32> {
        match self {
            WaitTarget::Job(job) => job
                .is_completed()
                .then(|| job.statuses().map_or(0, |statuses| statuses.status)),
            WaitTarget::Handle(handle) => handle.status(),
        }
    }

    /// Return whether the target is a wait handle that a wait consumes.
    fn is_handle(&self) -> bool {
        matches!(self, WaitTarget::Handle(_))
    }
}

/// Resolve a job spec `%N` to its job.
fn job_for_spec(parser: &Parser, spec: &wstr) -> Option<Rc<Job>> {
    let job_id = fish_wcstoi(&spec[1..]).ok()?;
    let job_id = if job_id == 0 {
        JobId::NONE
    } else {
        let job_id = u32::try_from(job_id).ok()?;
        let job_id = NonZeroU32::try_from(job_id).ok()?;
        MaybeJobId(Some(JobId::new(job_id)))
    };
    parser.job_with_id(job_id)
}

/// Wait for the targets, writing an identity/status pair for each observed event when `emit` is
/// set. Return the status of the last observed event.
fn wait_for_targets(
    parser: &mut Parser,
    streams: &mut IoStreams,
    targets: &[WaitTarget],
    any_flag: bool,
    emit: bool,
) -> BuiltinResult {
    if targets.is_empty() {
        return Ok(SUCCESS);
    }

    let mut observed = vec![false; targets.len()];
    let mut last_status = 0;
    let mut sigint = SigChecker::new_sighupintterm();

    loop {
        // Run tasks that can run and collect the jobs that completed before waiting for an event.
        parser.run_runnable_tasks();
        proc_clean_jobs(parser);
        // A reaped child can satisfy a blocked stage task; that task must run before the wait
        // sleeps, or the pipe it owns stays open and the waited job never completes.
        parser.wake_ready_tasks();
        parser.run_runnable_tasks();

        let mut any_observed = false;
        for (index, target) in targets.iter().enumerate() {
            if observed[index] {
                continue;
            }
            let Some(status) = target.status() else {
                continue;
            };
            observed[index] = true;
            any_observed = true;
            last_status = status;
            if emit {
                let mut record = target.identity();
                record.push('\n');
                record.push_utfstr(&status.to_wstring());
                record.push('\n');
                streams.out.append(&record);
            }
            if target.is_handle() {
                if let WaitTarget::Handle(handle) = target {
                    parser.mut_wait_handles().remove(handle);
                }
            } else if let WaitTarget::Job(job) = target {
                for process in job.external_procs() {
                    parser
                        .mut_wait_handles()
                        .remove_by_pid(process.pid().unwrap());
                }
            }
            if any_flag {
                return BuiltinResult::from_dynamic(last_status);
            }
        }

        if observed.iter().all(|&done| done) {
            return BuiltinResult::from_dynamic(last_status);
        }

        if any_observed {
            continue;
        }
        if sigint.check() {
            return Err(128 + libc::SIGINT);
        }

        // No target has completed. Wait for the next event, then run any task that became
        // runnable.
        proc_wait_any(parser);
    }
}

pub fn wait(parser: &mut Parser, streams: &mut IoStreams, argv: &mut [&wstr]) -> BuiltinResult {
    let cmd = argv[0];
    let argc = argv.len();
    let mut any_flag = false; // flag for -n option
    let mut emit = false;
    let mut print_help = false;
    let print_hints = false;

    let shortopts: &wstr = L!("nh");
    let longopts: &[WOption] = &[
        wopt(L!("any"), ArgType::NoArgument, 'n'),
        wopt(L!("emit"), ArgType::NoArgument, NON_OPTION_CHAR),
        wopt(L!("help"), ArgType::NoArgument, 'h'),
    ];

    // The single-dash form is not an option of this command: the short-option space carries no
    // meaning, so a cluster that names the long option is unknown.
    for arg in &argv[1..] {
        if &**arg == "--" {
            break;
        }
        if arg.char_at(0) == '-' && arg.char_at(1) != '-' && arg.chars().any(|c| c == 'e') {
            builtin_unknown_option(parser, streams, cmd, arg, print_hints);
            return Err(STATUS_INVALID_ARGS);
        }
    }

    let mut w = WGetopter::new(shortopts, longopts, argv);
    while let Some(c) = w.next_opt() {
        match c {
            'n' => {
                any_flag = true;
            }
            NON_OPTION_CHAR => {
                emit = true;
            }
            'h' => {
                print_help = true;
            }
            ':' => {
                builtin_missing_argument(
                    parser,
                    streams,
                    cmd,
                    None,
                    argv[w.wopt_index - 1],
                    print_hints,
                );
                return Err(STATUS_INVALID_ARGS);
            }
            ';' => {
                builtin_unexpected_argument(
                    parser,
                    streams,
                    cmd,
                    argv[w.wopt_index - 1],
                    print_hints,
                );
                return Err(STATUS_INVALID_ARGS);
            }
            '?' => {
                builtin_unknown_option(parser, streams, cmd, argv[w.wopt_index - 1], print_hints);
                return Err(STATUS_INVALID_ARGS);
            }
            _ => {
                panic!("unexpected retval from wgeopter.next()");
            }
        }
    }

    if print_help {
        builtin_print_help(parser, streams, cmd);
        return Ok(SUCCESS);
    }

    let mut targets: Vec<WaitTarget> = Vec::new();

    // Collect the jobs that already completed, so they are not observed as waitable.
    proc_clean_jobs(parser);

    if w.wopt_index == argc {
        // No operands: observe every waitable background job other than the job of the calling
        // task. A job whose task stage is the calling task cannot complete while that task waits.
        for job in parser.jobs() {
            if !can_wait_on_job(job) || parser.job_contains_calling_task(job) {
                continue;
            }
            targets.push(WaitTarget::Job(Rc::clone(job)));
        }
        // A child whose job entry is already collected is not observed again by a wait without
        // operands; its status was collected with the job.
        let target_jobs: Vec<InternalJobId> = targets
            .iter()
            .filter_map(|target| match target {
                WaitTarget::Job(job) => Some(job.internal_job_id),
                WaitTarget::Handle(_) => None,
            })
            .collect();
        for handle in parser.wait_handles().get_list() {
            if !target_jobs.contains(&handle.internal_job_id) {
                parser.mut_wait_handles().remove(&handle);
            }
        }
        return wait_for_targets(parser, streams, &targets, any_flag, emit);
    }

    let optind = w.wopt_index;
    for item in &argv[optind..argc] {
        if item.char_at(0) == '%' {
            let Some(job) = job_for_spec(parser, item) else {
                err_fmt!("No suitable job: %s", item)
                    .cmd(cmd)
                    .finish(streams);
                return Err(STATUS_INVALID_ARGS);
            };
            if parser.job_contains_calling_task(&job) {
                err_fmt!("%s: the calling task's own job", item)
                    .cmd(cmd)
                    .finish(streams);
                return Err(STATUS_INVALID_ARGS);
            }
            targets.push(WaitTarget::Job(job));
        } else if iswnumeric(item) {
            let Ok(pid) = parse_pid(streams, cmd, item) else {
                continue;
            };
            let mut handles = Vec::new();
            if !find_wait_handles(WaitHandleQuery::Pid(pid), parser, &mut handles) {
                err_fmt!("Could not find a job with process ID '%d'", pid,)
                    .cmd(cmd)
                    .finish(streams);
            }
            targets.extend(handles.into_iter().map(WaitTarget::Handle));
        } else {
            // Argument is a process name.
            let mut handles = Vec::new();
            if !find_wait_handles(WaitHandleQuery::ProcName(item), parser, &mut handles) {
                err_fmt!("Could not find child processes with the name '%s'", item,)
                    .cmd(cmd)
                    .finish(streams);
            }
            targets.extend(handles.into_iter().map(WaitTarget::Handle));
        }
    }
    if targets.is_empty() {
        return Err(STATUS_INVALID_ARGS);
    }
    wait_for_targets(parser, streams, &targets, any_flag, emit)
}
