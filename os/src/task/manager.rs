//!Implementation of [`TaskManager`]
use super::task::BIG_STRIDE;
use super::TaskControlBlock;
use crate::sync::UPSafeCell;
use alloc::collections::VecDeque;
use alloc::sync::Arc;
use lazy_static::*;
///A array of `TaskControlBlock` that is thread-safe
pub struct TaskManager {
    ready_queue: VecDeque<Arc<TaskControlBlock>>,
}

/// A simple FIFO scheduler.
impl TaskManager {
    ///Creat an empty TaskManager
    pub fn new() -> Self {
        Self {
            ready_queue: VecDeque::new(),
        }
    }
    /// Add process back to ready queue
    pub fn add(&mut self, task: Arc<TaskControlBlock>) {
        self.ready_queue.push_back(task);
    }
    /// Take the process with the smallest pass out of the ready queue.
    pub fn fetch(&mut self) -> Option<Arc<TaskControlBlock>> {
        let index = self
            .ready_queue
            .iter()
            .enumerate()
            .min_by_key(|(_, task)| task.inner_exclusive_access().pass)
            .map(|(index, _)| index)?;
        let task = self.ready_queue.remove(index)?;
        let mut inner = task.inner_exclusive_access();
        inner.pass = inner.pass.wrapping_add(inner.stride);
        drop(inner);
        Some(task)
    }

    /// Update priority and stride of a process. Priorities below 2 are invalid.
    pub fn set_priority(task: &Arc<TaskControlBlock>, priority: usize) -> isize {
        if priority < 2 {
            return -1;
        }
        let mut inner = task.inner_exclusive_access();
        inner.priority = priority;
        inner.stride = BIG_STRIDE / priority;
        priority as isize
    }
}

lazy_static! {
    /// TASK_MANAGER instance through lazy_static!
    pub static ref TASK_MANAGER: UPSafeCell<TaskManager> =
        unsafe { UPSafeCell::new(TaskManager::new()) };
}

/// Add process to ready queue
pub fn add_task(task: Arc<TaskControlBlock>) {
    //trace!("kernel: TaskManager::add_task");
    TASK_MANAGER.exclusive_access().add(task);
}

/// Take a process out of the ready queue
pub fn fetch_task() -> Option<Arc<TaskControlBlock>> {
    //trace!("kernel: TaskManager::fetch_task");
    TASK_MANAGER.exclusive_access().fetch()
}
