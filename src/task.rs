//! Tasks: the evaluation a shell schedules, the store it reads and the identity the kernel uses.
//!
//! A task is one evaluation together with the variables, function definitions and
//! current-directory descriptor it reads. The session task reads the shell's store; a task
//! started from it reads a copy, so a write in one is invisible to the other. The table assigns
//! generation-checked identities, so the identity of a finished task does not denote a later task
//! in the same slot.

use crate::env::{EnvMode, EnvSetMode, EnvStack};
use crate::function::FunctionStore;
use crate::io::{IoBufferfill, IoChain, IoFd};
use crate::parse_tree::ParsedSourceRef;
use crate::prelude::*;
use crate::proc::{InternalJobId, JobGroupRef};
use crate::task_frame::FrameStack;
use std::collections::VecDeque;
use std::os::fd::{AsRawFd as _, OwnedFd, RawFd};
use std::sync::Arc;

/// The lifecycle stage of a task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TaskLifecycle {
    /// The task can continue and waits for the scheduler.
    Runnable,
    /// The evaluator runs the task.
    Running,
    /// The task waits for an event.
    Blocked,
    /// The task waits for a resume operation.
    Suspended,
    /// The task's command list has completed.
    Finished,
}

/// The variables and function definitions a task reads.
pub(crate) struct TaskStore {
    variables: EnvStack,
    functions: FunctionStore,
}

impl TaskStore {
    /// Return the variables of this store.
    pub fn variables(&self) -> &EnvStack {
        &self.variables
    }

    /// Return the function definitions of this store.
    pub fn functions(&self) -> &FunctionStore {
        &self.functions
    }

    /// Return the function definitions for modification.
    pub fn functions_mut(&mut self) -> &mut FunctionStore {
        &mut self.functions
    }

    /// Return a copy for a task started from this store. The copy isolates variables and function
    /// definitions, and the task starts no process, so its `$last_pid` is empty.
    fn copy_for_task(&self) -> TaskStore {
        let variables = self.variables.copy_for_task(false);
        variables.set_empty(L!("last_pid"), EnvSetMode::new(EnvMode::GLOBAL, false));
        TaskStore {
            variables,
            functions: self.functions.copy_for_task(),
        }
    }
}

/// The descriptors recorded for a task, keyed by file descriptor.
///
/// Descriptors 0, 1 and 2 occupy fixed slots; every other descriptor is recorded in a vector
/// sorted by descriptor number. A table copy shares its entries until either side writes, and an
/// entry is an `Arc<OwnedFd>`, so a copy never duplicates a kernel descriptor.
#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Default)]
pub(crate) struct DescriptorTable(Arc<DescriptorTableEntries>);

#[cfg_attr(not(test), allow(dead_code))]
#[derive(Clone, Default)]
struct DescriptorTableEntries {
    standard: [Option<Arc<OwnedFd>>; 3],
    extra: Vec<(RawFd, Arc<OwnedFd>)>,
}

#[cfg_attr(not(test), allow(dead_code))]
impl DescriptorTable {
    /// Return the descriptor recorded for `fd`.
    pub fn get(&self, fd: RawFd) -> Option<&Arc<OwnedFd>> {
        if (0..3).contains(&fd) {
            return self.0.standard[fd as usize].as_ref();
        }
        let index = self
            .0
            .extra
            .binary_search_by_key(&fd, |(recorded_fd, _)| *recorded_fd)
            .ok()?;
        Some(&self.0.extra[index].1)
    }

    /// Record `descriptor` for `fd`, replacing the record already present.
    pub fn insert(&mut self, fd: RawFd, descriptor: Arc<OwnedFd>) {
        if (0..3).contains(&fd) {
            Arc::make_mut(&mut self.0).standard[fd as usize] = Some(descriptor);
            return;
        }
        let entries = Arc::make_mut(&mut self.0);
        match entries
            .extra
            .binary_search_by_key(&fd, |(recorded_fd, _)| *recorded_fd)
        {
            Ok(index) => entries.extra[index] = (fd, descriptor),
            Err(index) => entries.extra.insert(index, (fd, descriptor)),
        }
    }

    /// Remove the record for `fd`.
    pub fn remove(&mut self, fd: RawFd) {
        if (0..3).contains(&fd) {
            Arc::make_mut(&mut self.0).standard[fd as usize] = None;
            return;
        }
        let entries = Arc::make_mut(&mut self.0);
        if let Ok(index) = entries
            .extra
            .binary_search_by_key(&fd, |(recorded_fd, _)| *recorded_fd)
        {
            entries.extra.remove(index);
        }
    }

    /// Append every (descriptor number, descriptor) pair this table records.
    pub fn entries(&self) -> impl Iterator<Item = (RawFd, &Arc<OwnedFd>)> + '_ {
        let standard = self
            .0
            .standard
            .iter()
            .enumerate()
            .filter_map(|(index, entry)| {
                entry
                    .as_ref()
                    .map(|descriptor| (index as RawFd, descriptor))
            });
        standard.chain(
            self.0
                .extra
                .iter()
                .map(|(fd, descriptor)| (*fd, descriptor)),
        )
    }

    /// Record the descriptor that `chain` resolves for every descriptor number it redirects.
    pub fn record_from_chain(&mut self, chain: &IoChain) {
        for io in &chain.0 {
            let fd = io.fd();
            if let Some(descriptor) = chain.resolve_shared_fd(fd) {
                self.insert(fd, descriptor);
            }
        }
    }

    /// Return a chain that redirects every recorded descriptor to its source descriptor, so a
    /// process stage inherits the table without duplicating its descriptors.
    pub fn to_io_chain(&self) -> IoChain {
        let mut chain = IoChain::new();
        for (fd, descriptor) in self.entries() {
            let source_fd = descriptor.as_raw_fd();
            if source_fd != fd {
                chain.push(Arc::new(IoFd::new(fd, source_fd)));
            }
        }
        chain
    }
}

/// The descriptors recorded for a task: its current-directory descriptor and its descriptor table.
pub(crate) struct TaskDescriptors {
    /// The descriptor of the task's current directory.
    cwd_fd: Option<Arc<OwnedFd>>,
    /// The descriptors the task records by number.
    descriptor_table: DescriptorTable,
}

impl TaskDescriptors {
    /// Return the descriptor of the task's current directory.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cwd_fd(&self) -> Option<&Arc<OwnedFd>> {
        self.cwd_fd.as_ref()
    }

    /// Record the descriptor of the task's current directory.
    pub fn set_cwd_fd(&mut self, cwd_fd: Arc<OwnedFd>) {
        self.cwd_fd = Some(cwd_fd);
    }

    /// Return the task's descriptor table.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn descriptor_table(&self) -> &DescriptorTable {
        &self.descriptor_table
    }

    /// Return the task's descriptor table for modification.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn descriptor_table_mut(&mut self) -> &mut DescriptorTable {
        &mut self.descriptor_table
    }

    /// Return a copy for a task started from this one; descriptors are shared until a write.
    fn copy_for_task(&self) -> TaskDescriptors {
        TaskDescriptors {
            cwd_fd: self.cwd_fd.clone(),
            descriptor_table: self.descriptor_table.clone(),
        }
    }
}

/// The job stage a scheduled task evaluates.
pub(crate) struct StageTask {
    /// The descriptors the stage's evaluation reads.
    pub(crate) io_chain: IoChain,

    /// The job whose stage this task is.
    pub(crate) job: InternalJobId,

    /// The job's process group, which the stage's child jobs reuse.
    pub(crate) job_group: Option<JobGroupRef>,

    /// The buffer that holds output piped into a later stage.
    pub(crate) buffered_output: Option<Arc<IoBufferfill>>,

    /// The parsed source the stage's frames were built from.
    pub(crate) source: ParsedSourceRef,
}

/// One evaluation the shell schedules, with the store and descriptors it reads.
pub(crate) struct Task {
    store: TaskStore,
    descriptors: TaskDescriptors,
    frames: FrameStack,
    lifecycle: TaskLifecycle,
    stage: Option<StageTask>,
}

impl Task {
    /// Return a task that reads the shell's store.
    pub fn session(variables: EnvStack, cwd_fd: Option<Arc<OwnedFd>>) -> Task {
        Task {
            store: TaskStore {
                variables,
                functions: FunctionStore::shell(),
            },
            descriptors: TaskDescriptors {
                cwd_fd,
                descriptor_table: DescriptorTable::default(),
            },
            frames: FrameStack::new(),
            lifecycle: TaskLifecycle::Runnable,
            stage: None,
        }
    }

    /// Return a task started from this one: a copy of its store and descriptors.
    pub fn start_from(&self) -> Task {
        Task {
            store: self.store.copy_for_task(),
            descriptors: self.descriptors.copy_for_task(),
            frames: FrameStack::new(),
            lifecycle: TaskLifecycle::Runnable,
            stage: None,
        }
    }

    /// Return the store this task reads.
    pub fn store(&self) -> &TaskStore {
        &self.store
    }

    /// Return the store this task reads for modification.
    pub fn store_mut(&mut self) -> &mut TaskStore {
        &mut self.store
    }

    /// Return the descriptors recorded for this task.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn descriptors(&self) -> &TaskDescriptors {
        &self.descriptors
    }

    /// Return the descriptors recorded for this task for modification.
    pub fn descriptors_mut(&mut self) -> &mut TaskDescriptors {
        &mut self.descriptors
    }

    /// Return this task's stack of evaluation frames.
    pub fn frames_mut(&mut self) -> &mut FrameStack {
        &mut self.frames
    }

    /// Return the job stage this task evaluates.
    pub fn stage(&self) -> Option<&StageTask> {
        self.stage.as_ref()
    }

    /// Record the job stage this task evaluates.
    pub fn set_stage(&mut self, stage: StageTask) {
        self.stage = Some(stage);
    }

    /// Remove and return the job stage this task evaluates.
    pub fn take_stage(&mut self) -> Option<StageTask> {
        self.stage.take()
    }

    /// Return this task's lifecycle stage.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn lifecycle(&self) -> TaskLifecycle {
        self.lifecycle
    }

    /// Record this task's lifecycle stage.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_lifecycle(&mut self, lifecycle: TaskLifecycle) {
        self.lifecycle = lifecycle;
    }
}

/// The tasks that can run, in the order they were made runnable.
pub(crate) struct Scheduler {
    runnable: VecDeque<TaskId>,
}

impl Scheduler {
    /// Return a scheduler with no runnable task.
    pub fn new() -> Scheduler {
        Scheduler {
            runnable: VecDeque::new(),
        }
    }

    /// Record `id` as runnable.
    pub fn push(&mut self, id: TaskId) {
        self.runnable.push_back(id);
    }

    /// Return the next runnable task.
    pub fn pop(&mut self) -> Option<TaskId> {
        self.runnable.pop_front()
    }
}

/// The identity of one task. A slot's generation changes when the slot is reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TaskId {
    index: u32,
    generation: u32,
}

/// One slot of the task table, with the generation of the task recorded in it.
struct TaskSlot {
    generation: u32,
    task: Option<Task>,
}

/// The tasks the shell records, addressed by generation-checked identity.
pub(crate) struct TaskTable {
    slots: Vec<TaskSlot>,
    free: Vec<u32>,
}

impl TaskTable {
    /// Return a table that records `session`, together with the session task's identity.
    pub fn new(session: Task) -> (TaskTable, TaskId) {
        let mut table = TaskTable {
            slots: Vec::new(),
            free: Vec::new(),
        };
        let session_id = table.insert(session);
        (table, session_id)
    }

    /// Insert `task` and return its identity.
    pub fn insert(&mut self, task: Task) -> TaskId {
        let index = match self.free.pop() {
            Some(index) => {
                self.slots[index as usize].generation += 1;
                index
            }
            None => {
                let index = u32::try_from(self.slots.len()).expect("a task slot index fits u32");
                self.slots.push(TaskSlot {
                    generation: 1,
                    task: None,
                });
                index
            }
        };
        let slot = &mut self.slots[index as usize];
        slot.task = Some(task);
        TaskId {
            index,
            generation: slot.generation,
        }
    }

    /// Return the task `id` denotes, or `None` when the identity is stale.
    pub fn get(&self, id: TaskId) -> Option<&Task> {
        let slot = self.slots.get(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.task.as_ref()
    }

    /// Return the task `id` denotes for modification, or `None` when the identity is stale.
    pub fn get_mut(&mut self, id: TaskId) -> Option<&mut Task> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        slot.task.as_mut()
    }

    /// Remove the task `id` denotes, returning it. A later insertion may reuse the slot.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn remove(&mut self, id: TaskId) -> Option<Task> {
        let slot = self.slots.get_mut(id.index as usize)?;
        if slot.generation != id.generation {
            return None;
        }
        let task = slot.task.take();
        if task.is_some() {
            self.free.push(id.index);
        }
        task
    }
}

#[cfg(test)]
mod tests {
    use super::{DescriptorTable, Task, TaskLifecycle, TaskTable};
    use crate::env::{EnvMode, EnvSetMode, EnvStack, Environment as _};
    use crate::fds::make_autoclose_pipes;
    use crate::io::{IoChain, IoFd, IoPipe};
    use crate::tests::prelude::*;
    use fish_widestring::L;
    use std::os::fd::AsRawFd as _;
    use std::sync::Arc;

    #[test]
    #[serial]
    fn test_started_task_isolates_store_and_clears_last_pid() {
        test_init();
        let mut parser = TestParser::new();
        parser.vars().set_one(
            L!("task_probe"),
            EnvSetMode::default(),
            L!("parent").to_owned(),
        );
        parser.vars().set_one(
            L!("last_pid"),
            EnvSetMode::new(EnvMode::GLOBAL, false),
            L!("123").to_owned(),
        );
        let _ = parser.eval(
            L!("function task_probe_function; echo parent; end"),
            &IoChain::new(),
        );

        let task_id = parser.create_task();
        let task = parser.task(task_id).expect("the started task");
        let task_vars = task.store().variables();

        assert_eq!(
            task_vars.get(L!("task_probe")).unwrap().as_string(),
            L!("parent")
        );
        assert!(
            task.store()
                .functions()
                .get_props(L!("task_probe_function"))
                .is_some()
        );
        assert!(task_vars.get(L!("last_pid")).unwrap().is_empty());
        assert_eq!(
            parser.vars().get(L!("last_pid")).unwrap().as_string(),
            L!("123")
        );

        // A write in the task is invisible to the session task.
        task_vars.set_one(
            L!("task_probe"),
            EnvSetMode::default(),
            L!("task").to_owned(),
        );
        assert_eq!(
            parser.vars().get(L!("task_probe")).unwrap().as_string(),
            L!("parent")
        );

        // A later write in the session task is invisible to the task.
        parser.vars().set_one(
            L!("task_probe"),
            EnvSetMode::default(),
            L!("later").to_owned(),
        );
        assert_eq!(
            task_vars.get(L!("task_probe")).unwrap().as_string(),
            L!("task")
        );

        // The current-directory descriptor is shared with the started task.
        assert!(Arc::ptr_eq(
            parser
                .task_descriptors()
                .cwd_fd()
                .expect("the session current-directory descriptor"),
            task.descriptors()
                .cwd_fd()
                .expect("the task current-directory descriptor"),
        ));

        // The current task's descriptors are the base a started task inherits.
        parser.descriptor_table_mut().insert(
            3,
            Arc::new(make_autoclose_pipes().expect("a descriptor pipe").write),
        );
        let descriptor_task_id = parser.create_task();
        let descriptor_task = parser
            .task(descriptor_task_id)
            .expect("the task with descriptors");
        assert!(
            descriptor_task
                .descriptors()
                .descriptor_table()
                .get(3)
                .is_some()
        );

        parser.functions_mut().remove(L!("task_probe_function"));
    }

    #[test]
    fn test_descriptor_table_shares_entries_and_copies_on_write() {
        let first = make_autoclose_pipes().expect("the first pipe");
        let first_write_fd = first.write.as_raw_fd();
        let second = make_autoclose_pipes().expect("the second pipe");
        let second_write_fd = second.write.as_raw_fd();
        let mut table = DescriptorTable::default();
        table.insert(3, Arc::new(first.write));
        table.insert(5, Arc::new(first.read));
        table.insert(7, Arc::new(second.write));

        let mut copy = table.clone();
        assert_eq!(
            copy.get(5).unwrap().as_raw_fd(),
            table.get(5).unwrap().as_raw_fd()
        );
        copy.remove(5);
        assert!(copy.get(5).is_none());
        assert!(table.get(5).is_some());

        copy.insert(
            3,
            Arc::new(make_autoclose_pipes().expect("the replacement pipe").write),
        );
        assert_ne!(copy.get(3).unwrap().as_raw_fd(), first_write_fd);
        assert_eq!(table.get(3).unwrap().as_raw_fd(), first_write_fd);
        assert_eq!(table.get(7).unwrap().as_raw_fd(), second_write_fd);
    }

    #[test]
    fn test_descriptor_table_records_a_chain() {
        let pipes = make_autoclose_pipes().expect("a pipe");
        let write_fd = pipes.write.as_raw_fd();
        let mut chain = IoChain::new();
        chain.push(Arc::new(IoPipe::new(1, false, pipes.write)));
        chain.push(Arc::new(IoFd::new(2, 1)));

        let mut table = DescriptorTable::default();
        table.record_from_chain(&chain);
        assert_eq!(table.get(1).unwrap().as_raw_fd(), write_fd);
        assert_eq!(table.get(2).unwrap().as_raw_fd(), write_fd);

        let launch = table.to_io_chain();
        assert_eq!(launch.io_for_fd(1).unwrap().source_fd(), write_fd);
        assert_eq!(launch.io_for_fd(2).unwrap().source_fd(), write_fd);
        assert!(launch.io_for_fd(0).is_none());
    }

    #[test]
    fn test_task_table_rejects_stale_identity() {
        let (mut table, session_id) = TaskTable::new(Task::session(EnvStack::new(), None));
        assert!(table.get(session_id).is_some());

        let task_id = table.insert(Task::session(EnvStack::new(), None));
        assert!(table.get(task_id).is_some());
        assert!(table.remove(task_id).is_some());
        assert!(table.get(task_id).is_none());

        let reused_id = table.insert(Task::session(EnvStack::new(), None));
        assert_ne!(reused_id, task_id);
        assert!(table.get(reused_id).is_some());
        assert!(table.get(task_id).is_none());

        let task = table.get_mut(reused_id).expect("the reused slot task");
        for lifecycle in [
            TaskLifecycle::Runnable,
            TaskLifecycle::Running,
            TaskLifecycle::Blocked,
            TaskLifecycle::Suspended,
            TaskLifecycle::Finished,
        ] {
            task.set_lifecycle(lifecycle);
            assert_eq!(task.lifecycle(), lifecycle);
        }
    }
}
