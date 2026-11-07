# lab2 实验报告：地址空间与内存映射

## 1. 本实验完成的内容

本章在引入分页虚拟内存之后，重新实现了与用户地址空间交互的系统调用，并新增了匿名内存映射：

- 重写 `sys_get_time`，通过页表把 `TimeVal` 写回用户地址；
- 实现 `sys_trace`，支持按用户虚拟地址读/写一个字节，以及查询系统调用次数；
- 实现 `sys_mmap` 和 `sys_munmap`，支持按页申请和释放用户虚拟内存；
- 在 TCB 中维护每个任务的系统调用计数，并在 trap 入口统一计数。

主要修改文件：

- `os/src/syscall/process.rs`：四个系统调用的实现；
- `os/src/mm/memory_set.rs`：`MemorySet::mmap`、`MemorySet::munmap`；
- `os/src/task/task.rs`：TCB 增加 `syscall_times`；
- `os/src/task/mod.rs`：当前任务计数、mmap/munmap 的封装；
- `os/src/trap/mod.rs`：分派系统调用前完成计数。

## 2. `sys_get_time`

内核拿到的 `ts` 是用户虚拟地址，不能直接解引用。实现时先通过 `current_user_token()` 取得当前任务的页表 token，再用 `translated_byte_buffer` 把 `TimeVal` 的 16 个字节逐页写入用户空间。这样即使 `TimeVal` 横跨两个物理页，写入也不会越界。

时间来自 `timer::get_time_us()`，按 `CLOCK_FREQ` 换算成秒和微秒：

```rust
let time_val = TimeVal {
    sec: time_us / 1_000_000,
    usec: time_us % 1_000_000,
};
```

`get_time()` 在用户库中再换算成毫秒，`ch3_sleep` 和 `ch3_sleep1` 因此恢复正常。

## 3. `sys_trace`

`sys_trace` 有三种请求：

- `0`：读取用户地址 `id` 处的一个字节；
- `1`：把 `data` 的最低字节写入用户地址 `id`；
- `2`：返回系统调用 `id` 的累计次数；
- 其他值返回 `-1`。

读/写不能直接访问裸指针，而是先用当前页表把虚拟页号翻译成 PTE，检查 `V`、`R`、`W` 标志，再通过 `ppn().get_bytes_array()` 访问物理页内偏移。这样既能正确读写用户内存，也能在地址未映射或没有写权限时返回 `-1`，而不是让内核自己触发缺页。

计数放在 `TaskControlBlock::syscall_times` 中。trap 处理 `UserEnvCall` 时，先自增计数，再调用 `syscall()`。因此 `sys_trace` 查询自己的调用次数时，本次调用已经计入，这与测例的预期一致。

调试时遇到一个值得记录的边界情况：`isize::MAX` 直接按 39 位掩码后，页号恰好与 trampoline 页相同，导致 `trace_read(isize::MAX)` 返回了 trampoline 页中的字节。修复方式是在访问前拒绝 `addr >= 1 << 38` 的非用户地址，再查页表。`0x80200000` 这类内核地址本来就不在用户页表中，查表会自然失败。

## 4. `mmap` 与 `munmap`

`mmap(start, len, port)` 的约束：

- `start` 必须按页对齐，`len` 不能为 0；
- `port` 的第 0、1、2 位分别对应 `R`、`W`、`X`，`port == 0` 或 `port > 0b111` 返回 `-1`；
- 新区间不能与已有 `MapArea` 重叠；
- 成功时把 `MapPermission::U` 加上对应权限，插入一个 `Framed` 区域，返回 `0`。

权限转换需要注意 `port` 与 `MapPermission` 的位定义不同：

```rust
if port & 0b001 != 0 { permission |= MapPermission::R; }
if port & 0b010 != 0 { permission |= MapPermission::W; }
if port & 0b100 != 0 { permission |= MapPermission::X; }
```

因此 `port = 1` 只映射可读页，用户写会触发 Store Page Fault；`port = 2` 只映射可写页，RISC-V 不允许 `W=1, R=0`，读也会触发异常。这与 `ch4_mmap1`、`ch4_mmap2` 的预期一致。

`munmap(start, len)` 按题意只处理“唯一且完整”的映射：要求 `start` 页对齐、`len` 是页大小整数倍，并且在 `areas` 中找到起点和终点都完全匹配的区域。找到后调用 `MapArea::unmap`，同时移除 `data_frames` 中的 `FrameTracker`，物理页随之释放；找不到返回 `-1`。这样 `munmap(start, len + 1)` 和 `munmap(start + 1, len - 1)` 都会失败。

## 5. 测试结果

使用官方检查器执行：

```sh
make -C ci-user test CHAPTER=4
```

测例输出包含：

```text
Test 04_1 OK!
Test 04_4 test OK!
Test 04_5 ummap OK!
Test 04_6 ummap2 OK!
Test trace_1 OK!
Test trace OK!
Test sleep OK!
Test sleep1 passed!
```

官方汇总为 `Test passed: 16/16`。`ch4_mmap1`、`ch4_mmap2` 按预期触发了 PageFault，`ch4_unmap2` 的错误参数也正确返回了 `-1`。

## 6. 实验分析

本实验最关键的变化是“内核不能相信用户指针”。在开启页表后，内核看到的用户地址只是当前地址空间中的一个虚拟地址，必须经过 `PageTable::from_token` 翻译，并且要检查页表项权限，否则会绕过用户态保护。`sys_trace` 的越权检查、`sys_get_time` 的跨页写入以及 `mmap` 的权限映射，都是这一点的直接体现。

另一个容易忽略的问题是地址空间中的“空洞”和“边界”。`isize::MAX` 的掩码问题说明，仅仅依赖 39 位地址截断是不够的，还需要显式区分用户地址下界和内核地址；否则一个非法的用户地址可能恰好命中 trampoline 或其它内核映射。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾与 deepseek-v4.1-flash 就实验实现与调试进行交流，并在代码对应位置记录了参考来源。
2. 此外，我也参考了 rCore-Tutorial-Book-v3 与 rCore-Tutorial-Guide 的相关章节。
3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。
4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。
