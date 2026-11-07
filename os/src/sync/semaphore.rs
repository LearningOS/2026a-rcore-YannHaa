//! Semaphore

use crate::sync::UPSafeCell;
use crate::task::{
    block_current_and_run_next, current_process, current_task, wakeup_task, TaskControlBlock,
    TaskStatus, WaitResource,
};
use alloc::vec;
use alloc::{collections::VecDeque, sync::Arc, vec::Vec};

/// semaphore structure
pub struct Semaphore {
    /// id of this semaphore in its process
    pub id: usize,
    /// semaphore inner
    pub inner: UPSafeCell<SemaphoreInner>,
}

/// inner of semaphore
pub struct SemaphoreInner {
    /// available resource count
    pub count: isize,
    /// blocked tasks
    pub wait_queue: VecDeque<Arc<TaskControlBlock>>,
    /// resources currently held by each thread: (tid, count)
    pub owners: Vec<(usize, usize)>,
}

impl Semaphore {
    /// Create a new semaphore
    pub fn new(res_count: usize, id: usize) -> Self {
        trace!("kernel: Semaphore::new");
        Self {
            id,
            inner: unsafe {
                UPSafeCell::new(SemaphoreInner {
                    count: res_count as isize,
                    wait_queue: VecDeque::new(),
                    owners: Vec::new(),
                })
            },
        }
    }

    fn add_owner(inner: &mut SemaphoreInner, tid: usize, count: usize) {
        if let Some((_, held)) = inner.owners.iter_mut().find(|(owner, _)| *owner == tid) {
            *held += count;
        } else {
            inner.owners.push((tid, count));
        }
    }

    fn release_owner(inner: &mut SemaphoreInner, tid: usize) {
        if let Some(index) = inner.owners.iter().position(|(owner, _)| *owner == tid) {
            if inner.owners[index].1 <= 1 {
                inner.owners.remove(index);
            } else {
                inner.owners[index].1 -= 1;
            }
        }
    }

    /// up operation of semaphore
    pub fn up(&self) {
        trace!("kernel: Semaphore::up");
        let task = current_task().unwrap();
        let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
        let mut inner = self.inner.exclusive_access();
        inner.count += 1;
        Self::release_owner(&mut inner, tid);
        if inner.count <= 0 {
            if let Some(task) = inner.wait_queue.pop_front() {
                wakeup_task(task);
            }
        }
    }

    /// down operation of semaphore
    pub fn down(&self) {
        trace!("kernel: Semaphore::down");
        let task = current_task().unwrap();
        let tid = task.inner_exclusive_access().res.as_ref().unwrap().tid;
        let mut inner = self.inner.exclusive_access();
        if inner.count > 0 {
            inner.count -= 1;
            Self::add_owner(&mut inner, tid, 1);
            return;
        }
        drop(inner);

        if Self::would_deadlock(self.id, tid) {
            task.inner_exclusive_access().deadlock_detected = true;
            return;
        }

        let mut inner = self.inner.exclusive_access();
        inner.count -= 1;
        task.inner_exclusive_access().waiting_on = Some(WaitResource::Semaphore(self.id));
        inner.wait_queue.push_back(task.clone());
        drop(inner);
        block_current_and_run_next();
        task.inner_exclusive_access().waiting_on = None;
        let mut inner = self.inner.exclusive_access();
        Self::add_owner(&mut inner, tid, 1);
    }

    /// Check whether letting the current thread wait for this semaphore would deadlock.
    fn would_deadlock(sem_id: usize, tid: usize) -> bool {
        let process = current_process();
        let process_inner = process.inner_exclusive_access();
        if !process_inner.deadlock_detect_enabled {
            return false;
        }
        let sem_count = process_inner.semaphore_list.len();
        let task_count = process_inner.tasks.len();
        let mut available = vec![0isize; sem_count];
        let mut allocations = vec![vec![0isize; sem_count]; task_count];
        for (index, slot) in process_inner.semaphore_list.iter().enumerate() {
            if let Some(sem) = slot {
                let inner = sem.inner.exclusive_access();
                available[index] = if inner.count > 0 { inner.count } else { 0 };
                for &(owner, count) in inner.owners.iter() {
                    if owner < task_count {
                        allocations[owner][index] += count as isize;
                    }
                }
            }
        }
        // request[tid] = None means the thread can run and eventually release resources.
        let mut requests: Vec<Option<usize>> = vec![None; task_count];
        let mut done = vec![false; task_count];
        for (index, slot) in process_inner.tasks.iter().enumerate() {
            match slot {
                Some(task) => {
                    let inner = task.inner_exclusive_access();
                    if inner.res.is_none() {
                        done[index] = true;
                    } else if index == tid {
                        requests[index] = Some(sem_id);
                    } else if inner.task_status == TaskStatus::Blocked {
                        requests[index] = match inner.waiting_on {
                            Some(WaitResource::Semaphore(id)) => Some(id),
                            Some(_) => Some(usize::MAX),
                            None => None,
                        };
                    }
                }
                None => done[index] = true,
            }
        }
        loop {
            let mut progress = false;
            for index in 0..task_count {
                if done[index] {
                    continue;
                }
                let can_run = match requests[index] {
                    None => true,
                    Some(id) if id < sem_count => available[id] > 0,
                    _ => false,
                };
                if can_run {
                    if let Some(id) = requests[index] {
                        if id < sem_count {
                            available[id] -= 1;
                        }
                    }
                    for sem in 0..sem_count {
                        available[sem] += allocations[index][sem];
                    }
                    done[index] = true;
                    progress = true;
                }
            }
            if !progress {
                break;
            }
        }
        done.iter().any(|finished| !finished)
    }
}
