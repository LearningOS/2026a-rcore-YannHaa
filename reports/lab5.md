# lab5 实验报告：并发、同步与死锁检测

## 1. 本实验完成的内容

本章在多线程进程模型上补齐了同步资源的死锁检测，同时把前面各章的实验内容移植到 ch8 的进程/线程结构上：

- 实现 `sys_enable_deadlock_detect`；
- 为阻塞互斥锁增加持有者记录，检测同一线程重复加锁造成的自死锁；
- 为信号量增加持有者记录，在 `down` 真正阻塞前进行安全性检查；
- 在任务控制块中记录 `waiting_on`、`deadlock_detected`，在线程创建、唤醒、退出时正确维护；
- 保留 `sys_get_time`、`sys_mmap`、`sys_munmap`、`sys_spawn`、`sys_set_priority` 以及 ch6 的 `fstat`、`linkat`、`unlinkat`。

主要修改文件：

- `os/src/syscall/sync.rs`：`sys_enable_deadlock_detect`，`mutex_lock`/`semaphore_down` 的 `-0xdead` 返回值；
- `os/src/sync/mutex.rs`：`MutexBlocking` 的 `owner` 与自死锁检测；
- `os/src/sync/semaphore.rs`：资源持有者记录与安全性检查；
- `os/src/sync/condvar.rs`：记录条件变量等待状态；
- `os/src/task/task.rs`：`waiting_on`、`deadlock_detected`、`priority`/`stride`/`pass`；
- `os/src/task/process.rs`：进程级 `deadlock_detect_enabled` 和 `spawn`；
- `os/src/task/manager.rs`：stride 调度；
- `os/src/task/mod.rs`：`take_deadlock_detected`、`set_current_priority`。

## 2. 死锁检测的设计

### 2.1 记录线程状态

每个 `TaskControlBlockInner` 增加：

```rust
pub waiting_on: Option<WaitResource>,
pub deadlock_detected: bool,
```

`WaitResource` 区分 `Mutex`、`Semaphore`、`Condvar`。当线程调用 `mutex_lock`、`semaphore_down` 或 `condvar_wait` 并真正要阻塞时，先把 `waiting_on` 设为对应资源；被唤醒后清空。这样死锁检查可以区分“在等同步资源”和“在等定时器睡眠”的线程。

### 2.2 互斥锁的自死锁

`MutexBlockingInner` 增加 `owner: Option<usize>`。加锁成功时记录当前 tid；解锁时清空，若有等待者则把锁直接移交。

同一线程第二次锁同一个互斥锁时：

```rust
if detect && mutex_inner.owner == Some(tid) {
    task.inner_exclusive_access().deadlock_detected = true;
    return;
}
```

不进入阻塞队列，直接让 `sys_mutex_lock` 返回 `-0xdead`。`ch8_deadlock_mutex1` 正是这个场景。

### 2.3 信号量的安全性检查

信号量支持多个实例，不能用简单的“等待图找环”判断。`ch8_deadlock_sem2` 就是一个反例：图中可能存在环，但某个环外线程仍会释放资源，因此并不是死锁。本实验采用类似银行家算法的安全性检查：

1. 统计每个信号量当前可用数量 `available`（`count > 0` 时取 `count`，否则为 0）；
2. 统计每个线程持有的资源 `allocations`（`down` 成功时加一，`up` 时减一）；
3. 当前线程将要等待 `sem_id`，把它标记为“请求该信号量”；
4. 反复扫描所有线程：
   - 没有等待同步资源的线程（包括正在睡眠的线程）视为可以继续运行并最终释放资源；
   - 等待信号量的线程只有在 `available[id] > 0` 时才能继续；
   - 能继续的线程把持有的资源加回 `available`，标记为完成；
5. 如果一轮扫描没有任何进展，仍有线程未完成，则说明剩余线程互相等待，返回 `-0xdead`。

对应实现位于 `Semaphore::would_deadlock`。检查发生在真正 `count -= 1` 和进入等待队列之前；一旦判定会死锁，只设置 `deadlock_detected`，不修改信号量计数，也不阻塞。

### 2.4 为什么不能只看等待环

`ch8_deadlock_sem2` 中，线程 1 等待 `s1`，线程 3 等待 `s2`，而 `s2` 同时被线程 1 和线程 4 持有。等待图会经过“线程 1 等 `s1`、线程 3 等 `s2`”形成环，但线程 4 并不等待任何资源，它会释放 `s2`，因此不会死锁。安全性检查把“可运行的线程最终会释放资源”纳入计算，因此能正确区分这两种情况，`sem2` 的所有线程都以 0 退出。

## 3. 系统调用层

`sys_enable_deadlock_detect(enabled)` 只修改当前进程的标志：

```rust
process.inner_exclusive_access().deadlock_detect_enabled = enabled != 0;
```

`sys_mutex_lock` 和 `sys_semaphore_down` 在阻塞操作返回后检查并清除标志：

```rust
mutex.lock();
if take_deadlock_detected() { -0xdead } else { 0 }
```

`take_deadlock_detected` 读取后立即清零，避免一次检测影响后续系统调用。`-0xdead` 即 `-57005`，与用户库中的判断一致。

## 4. 测试结果

使用官方检查器：

```sh
make -C ci-user test CHAPTER=8
```

测试汇总为 `Test passed: 25/25`，与并发和同步相关的输出包括：

```text
deadlock test mutex 1 OK!
deadlock test semaphore 1 OK!
deadlock test semaphore 2 OK!
mpsc_sem passed!
philosopher dining problem with mutex test passed!
race adder using spin mutex test passed!
sync_sem passed!
test_condvar passed!
threads with arg test passed!
threads test passed!
pipetest passed!
```

其中 `ch8_deadlock_sem1` 至少有一个线程返回非 0，`ch8_deadlock_sem2` 的四个线程全部返回 0，说明检测既能发现真实死锁，也没有对多实例资源的“假环”误报。

## 5. 实验分析

死锁检测的难点不在于“找到环”，而在于准确描述“资源还会不会被释放”。对于单实例的互斥锁，持有者关系就足够；对于多实例的信号量，必须同时考虑可用数量、每个线程的持有量以及哪些线程仍然可以运行。银行家式的安全性检查用“能否找到一个完成顺序”代替简单的图环判断，因此能处理 `sem2` 这种多实例场景。

另一个实现细节是 `waiting_on` 的清理时机。线程被 `wakeup_task` 唤醒后状态变为 `Ready`，但它可能还没有真正运行，`waiting_on` 仍是旧值。因此安全性检查必须结合 `task_status`：只有 `Blocked` 且 `waiting_on` 指向同步资源的线程才算资源阻塞；`Ready`/`Running` 的线程即使 `waiting_on` 尚未清空，也应视为可运行。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾与 deepseek-v4.1-flash 就实验实现与调试进行交流，并在代码对应位置记录了参考来源。
2. 此外，我也参考了 rCore-Tutorial-Book-v3、rCore-Tutorial-Guide 以及 rCore 的线程与同步章节。
3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。
4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。
