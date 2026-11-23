//! Tasks: the evaluation a shell schedules, the store it reads and the identity the kernel uses.
//!
//! A task is one evaluation together with the variables, function definitions and
//! current-directory descriptor it reads. The session task reads the shell's store; a task
//! started from it reads a copy, so a write in one is invisible to the other. The table assigns
//! generation-checked identities, so the identity of a finished task does not denote a later task
//! in the same slot.

use crate::env::{EnvMode, EnvSetMode, EnvStack};
use crate::function::FunctionStore;
use crate::prelude::*;
use std::os::fd::OwnedFd;
use std::sync::Arc;

/// The lifecycle stage of a task.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) enum TaskState {
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

/// The task-local data that is not a variable.
pub(crate) struct TaskLocalData {
    /// The descriptor of the task's current directory.
    cwd_fd: Option<Arc<OwnedFd>>,
}

impl TaskLocalData {
    /// Return the descriptor of the task's current directory.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn cwd_fd(&self) -> Option<&Arc<OwnedFd>> {
        self.cwd_fd.as_ref()
    }

    /// Record the descriptor of the task's current directory.
    pub fn set_cwd_fd(&mut self, cwd_fd: Arc<OwnedFd>) {
        self.cwd_fd = Some(cwd_fd);
    }

    /// Return a copy for a task started from this one; the descriptor is shared.
    fn copy_for_task(&self) -> TaskLocalData {
        TaskLocalData {
            cwd_fd: self.cwd_fd.clone(),
        }
    }
}

/// One evaluation the shell schedules, with the store and task-local data it reads.
pub(crate) struct Task {
    store: TaskStore,
    local: TaskLocalData,
    state: TaskState,
}

impl Task {
    /// Return a task that reads the shell's store.
    pub fn session(variables: EnvStack, cwd_fd: Option<Arc<OwnedFd>>) -> Task {
        Task {
            store: TaskStore {
                variables,
                functions: FunctionStore::shell(),
            },
            local: TaskLocalData { cwd_fd },
            state: TaskState::Runnable,
        }
    }

    /// Return a task started from this one: a copy of its store and task-local data.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn start_from(&self) -> Task {
        Task {
            store: self.store.copy_for_task(),
            local: self.local.copy_for_task(),
            state: TaskState::Runnable,
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

    /// Return this task's task-local data.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn local(&self) -> &TaskLocalData {
        &self.local
    }

    /// Return this task's task-local data for modification.
    pub fn local_mut(&mut self) -> &mut TaskLocalData {
        &mut self.local
    }

    /// Return this task's lifecycle stage.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn state(&self) -> TaskState {
        self.state
    }

    /// Record this task's lifecycle stage.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn set_state(&mut self, state: TaskState) {
        self.state = state;
    }
}

/// The identity of one task. A slot's generation changes when the slot is reused.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TaskId {
    index: u32,
    generation: u32,
}

/// One slot of the task table, with the generation of the task it holds.
struct TaskSlot {
    generation: u32,
    task: Option<Task>,
}

/// The tasks the shell holds, addressed by generation-checked identity.
pub(crate) struct TaskTable {
    slots: Vec<TaskSlot>,
    free: Vec<u32>,
}

impl TaskTable {
    /// Return a table holding `session` together with the identity of the session task.
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
    use super::{Task, TaskState, TaskTable};
    use crate::env::{EnvMode, EnvSetMode, EnvStack, Environment as _};
    use crate::io::IoChain;
    use crate::tests::prelude::*;
    use fish_widestring::L;
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
                .local()
                .cwd_fd()
                .expect("the session current-directory descriptor"),
            task.local()
                .cwd_fd()
                .expect("the task current-directory descriptor"),
        ));

        parser.functions_mut().remove(L!("task_probe_function"));
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
        for state in [
            TaskState::Runnable,
            TaskState::Running,
            TaskState::Blocked,
            TaskState::Suspended,
            TaskState::Finished,
        ] {
            task.set_state(state);
            assert_eq!(task.state(), state);
        }
    }
}
