# lab3 实验报告：进程管理与调度

## 1. 本实验完成的内容

本章把内核从“只能加载静态应用”推进到“可以动态创建进程”，并实现了带优先级的调度：

- `sys_spawn`：按 ELF 文件创建全新进程；
- `sys_set_priority`：修改当前进程优先级，并返回设置后的优先级；
- stride 调度器：根据优先级给每个进程分配不同的运行份额；
- 同时保留上一章的 `sys_get_time`、`sys_mmap`、`sys_munmap`，使 ch4 测例继续通过。

主要修改文件：

- `os/src/task/task.rs`：`TaskControlBlock::spawn`，TCB 增加 `priority`、`stride`、`pass`；
- `os/src/task/manager.rs`：选择 `pass` 最小的任务，并维护 `pass += stride`；
- `os/src/task/mod.rs`：导出 `set_current_priority`；
- `os/src/syscall/process.rs`：实现 `spawn`、`set_priority` 以及前一章的三个系统调用；
- `os/src/mm/memory_set.rs`：`mmap`、`munmap`。

## 2. `spawn` 的实现

`spawn` 与 `fork` 的区别在于：

- `fork` 复制父进程的整个用户地址空间，子进程从父进程的当前位置继续执行；
- `spawn` 根据可执行文件名重新读取 ELF，为子进程建立全新的地址空间，从 ELF 入口开始执行。

实现流程：

1. 用 `translated_str` 从用户空间读出路径字符串；
2. 调用 `get_app_data_by_name` 取得 ELF 数据；
3. 用 `MemorySet::from_elf` 建立新的用户地址空间；
4. 分配新的 PID 和内核栈；
5. 复制父进程的文件描述符表，使子进程可以直接使用标准输入输出；
6. 把子进程挂到父进程的 `children` 链表上；
7. 构造 `TrapContext`，入口设为 ELF entry，`kernel_sp` 设为新内核栈顶；
8. `add_task` 把子进程的主线程加入就绪队列。

`ch5_spawn0` 连续创建 40 个 `ch5_getpid` 子进程，`ch5_spawn1` 检查 `wait`/`waitpid` 能否拿到正确的退出码，均通过。

## 3. stride 调度

每个任务在 TCB 中增加三个字段：

- `priority`：优先级，越大获得的 CPU 份额越多；
- `stride`：每次被调度后 `pass` 的增量，`stride = BIG_STRIDE / priority`；
- `pass`：累计“虚拟运行时间”，调度器每次选择 `pass` 最小的任务。

常量取 `BIG_STRIDE = 1 << 20`，默认优先级为 16。`fetch` 在就绪队列中线性扫描最小 `pass`：

```rust
let index = self.ready_queue.iter().enumerate()
    .min_by_key(|(_, task)| task.inner_exclusive_access().pass)
    .map(|(index, _)| index)?;
let task = self.ready_queue.remove(index)?;
let mut inner = task.inner_exclusive_access();
inner.pass = inner.pass.wrapping_add(inner.stride);
```

`sys_set_priority` 的约束来自测例：

- `10`、`isize::MAX` 是合法优先级，返回原值；
- `0`、`1`、`-10` 非法，返回 `-1`。

因此实现中先判断 `prio < 2`，再把 `prio as usize` 交给 `TaskManager::set_priority`。设置优先级时更新 `priority` 和 `stride`，保留当前 `pass`，使进程不会因为一次优先级调整而被无条件插队。

## 4. 测试结果

使用官方检查器：

```sh
make -C ci-user test CHAPTER=5
```

测试汇总为 `Test passed: 15/15`，包括：

```text
get_time OK!
Test sleep OK!
Test 04_1 OK!
Test 04_4 test OK!
Test 04_5 ummap OK!
Test 04_6 ummap2 OK!
Test spawn0 OK!
Test wait OK!
Test waitpid OK!
Test set_priority OK!
```

`ch5_stride` 在 4 秒窗口内比较了优先级 5 到 10 的六个进程，实际输出：

```text
priority = 5,  exitcode = 7761600,  ratio = 1552320
priority = 6,  exitcode = 9501600,  ratio = 1583600
priority = 7,  exitcode = 10935600, ratio = 1562228
priority = 8,  exitcode = 12697200, ratio = 1587150
priority = 9,  exitcode = 14136400, ratio = 1570711
priority = 10, exitcode = 15414000, ratio = 1541400
```

`count / priority` 基本保持在 155 万左右，说明运行时间与优先级近似成正比，stride 调度达到了预期效果。

## 5. 实验分析

stride 调度的好处是实现简单且能给出确定的份额：每个进程的 `pass` 增长速度与优先级成反比，调度器只需要每次选择最小值，不需要维护复杂的时间片轮转队列。需要注意的是 `pass` 可能溢出，因此使用 `wrapping_add`；`priority` 取 `isize::MAX` 时 `stride` 会变成 0，该进程会长期占据 CPU，但测例只检查返回值，随后立即退出，因此不会造成实际影响。

`spawn` 的实现说明了进程与线程资源的分离：进程拥有地址空间、文件描述符和子进程关系，线程拥有用户栈、trap 上下文和内核栈。`spawn` 创建的是新进程的主线程，所以既要新建地址空间，也要新建内核栈，不能复用父进程的 `TrapContext`。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾与 deepseek-v4.1-flash 就实验实现与调试进行交流，并在代码对应位置记录了参考来源。
2. 此外，我也参考了 rCore-Tutorial-Book-v3 与 rCore-Tutorial-Guide 的相关章节。
3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。
4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。
