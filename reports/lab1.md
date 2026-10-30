# lab1 实验报告

## 实现功能：sys_trace

实现了 `sys_trace` 系统调用。根据 `trace_request` 的不同，它分别完成：

- `0`：读取地址 `id` 处的一个字节并返回；
- `1`：把 `data` 的最低一个字节写入地址 `id`，成功返回 `0`；
- `2`：返回当前任务调用系统调用 `id` 的次数；
- 其他请求值：返回 `-1`。

每个任务在 TCB 中独立维护系统调用计数。统一系统调用入口在分派具体处理函数之前先执行计数，因此查询 `sys_trace` 自身调用次数时，本次调用也已经被计入。

## U 态权限隔离测试

### 测试方法

使用第 3 章内核单独构建并运行以下三个测例：

- `ch2b_bad_address.rs`
- `ch2b_bad_instructions.rs`
- `ch2b_bad_register.rs`

运行命令如下：

```sh
make -C os run OFFLINE=1 TEST=2 BASE=1 \
  APPS='src/bin/ch2b_bad_address.rs src/bin/ch2b_bad_instructions.rs src/bin/ch2b_bad_register.rs' \
  OBJCOPY=/home/ubuntu/.cargo/bin/rust-objcopy \
  OBJDUMP=/home/ubuntu/.cargo/bin/rust-objdump
```

### 实际输出

```text
[kernel] Hello, world!
[kernel] PageFault in application, bad addr = 0x0, bad instruction = 0x804003a4, kernel killed it.
[kernel] IllegalInstruction in application, kernel killed it.
[kernel] IllegalInstruction in application, kernel killed it.
[kernel] Panicked at src/task/mod.rs:156 All applications completed!
```

### 现象分析

1. `ch2b_bad_address` 在 U 态向地址 `0x0` 写入数据。U 态程序没有访问该地址的权限，硬件产生访存异常。内核捕获后输出 `PageFault in application`，其中坏地址为 `0x0`，出错指令地址为 `0x804003a4`，随后终止该应用。

2. `ch2b_bad_instructions` 在 U 态执行 `sret` 指令。`sret` 是 S 态特权指令，在 U 态执行会触发 `IllegalInstruction` 异常，而不会真的完成特权级切换。内核捕获异常并终止该应用。

3. `ch2b_bad_register` 在 U 态通过 `csrr` 读取 `sstatus`。`sstatus` 是 S 特权级 CSR，U 态无权读取，因此硬件同样触发 `IllegalInstruction` 异常。程序没有机会输出读到的值，随后被内核终止。

三个应用被终止后，调度器继续运行或结束其他任务。最后的
`All applications completed!` 是所有应用都已结束时的正常退出路径，不是上述测例失败造成的额外异常。

这些结果说明，进入 U 态后，程序不仅不能直接访问不属于自己的物理内存，也不能执行 S 态特权指令或访问 S 态 CSR；越权操作由硬件转换为 trap，交由 S 态内核处理。

## trap.S 与特权级切换

### `__alltraps` 和 `__restore` 的总体作用

`__alltraps` 是 trap 的统一入口。发生异常或中断时，硬件把 PC 设置为
`stvec` 中的地址，也就是 `__alltraps`。它完成以下工作：

1. 交换 `sp` 和 `sscratch`，从用户栈切换到内核栈；
2. 在 kernel stack 上分配一个 `TrapContext`；
3. 保存除 `x0`、`x2`、`x4` 外的通用寄存器；
4. 保存 `sstatus`、`sepc` 和用户栈指针；
5. 把 `TrapContext` 的地址放入 `a0`，调用 Rust 函数 `trap_handler`。

`__restore` 与 `__alltraps` 的工作方向相反。它从内核栈上的
`TrapContext` 恢复 CSR、用户栈指针和通用寄存器，释放 trap 上下文，再执行
`sret` 回到用户态。

### L40：刚进入 `__restore` 时 `sp` 的含义

刚进入 `__restore` 时，`sp` 指向 kernel stack 上保存的 `TrapContext`
的起始地址，也就是其中 `x[0]` 对应的位置。后面的 `32*8(sp)`、
`33*8(sp)` 等偏移都相对于这个地址。

`__restore` 主要有两种使用情景：

1. 任务第一次运行前。`TaskContext::goto_restore` 把 `ra` 设置为
   `__restore`，把 `sp` 设置为预先构造好的 `TrapContext` 地址；任务切换
   后从 `__restore` 第一次进入用户态。
2. 一次 trap 处理结束后。`__alltraps` 调用的 `trap_handler` 返回后，
   执行流继续进入其后的 `__restore`，恢复被打断任务的上下文并返回 U 态。

### L43-L48：三个特殊值的含义

```asm
ld t0, 32*8(sp)
ld t1, 33*8(sp)
ld t2, 2*8(sp)
csrw sstatus, t0
csrw sepc, t1
csrw sscratch, t2
```

- `32*8(sp)` 保存的是 `sstatus`。`sret` 会根据其中的 `SPP` 决定返回哪个
  特权级，并根据 `SPIE` 恢复中断使能状态。第一次进入用户态以及从 U 态
  陷入时，`SPP` 都是 U，因此恢复后执行 `sret` 会返回 U 态。
- `33*8(sp)` 保存的是 `sepc`。第一次进入用户态时它是应用入口地址；
  trap 返回时它是用户程序下一条应当执行的指令地址。例如系统调用
  `trap_handler` 会把保存的 `sepc` 加 4，使 `sret` 返回到 `ecall` 之后。
- `2*8(sp)` 保存的是用户栈指针 `sp`。把它写入 `sscratch`，是为了在
  L60 中用一条原子交换指令同时恢复用户 `sp` 并把内核栈指针留在
  `sscratch` 中，供下一次 trap 使用。

此时 `t0`、`t1`、`t2` 可以安全使用，因为它们虽然还没有恢复为用户程序
原来的值，但它们的内容已经保存在内核栈的 `TrapContext` 中。

### L50-L56：跳过 `x2` 和 `x4` 的原因

- `x2` 是 `sp`。此时 `sp` 仍指向内核栈上的 `TrapContext`，不能提前覆盖；
  用户 `sp` 已经存入 `sscratch`，会在 L60 通过交换恢复。
- `x4` 是 `tp`，当前用户程序不使用线程局部存储，trap 入口也没有保存它，
  因此无需从 `TrapContext` 恢复。
- `x0` 恒为零，也不需要通过访存恢复。

所以代码只恢复 `x1`、`x3` 以及 `x5` 到 `x31`。

### L60：`sp` 和 `sscratch` 的交换结果

```asm
csrrw sp, sscratch, sp
```

`csrrw rd, csr, rs1` 的语义是 `rd = csr`、`csr = rs1`。执行前，
`sp` 指向 kernel stack，`sscratch` 保存用户栈指针；执行后：

- `sp` 是用户栈指针，`sret` 后用户程序可以直接使用它；
- `sscratch` 是 kernel stack 指针（准确地说，是释放
  `TrapContext` 后原来的内核栈顶），下一次从 U 态 trap 时供
  `__alltraps` 的 L13 交换回内核栈。

### 状态切换发生的指令

在 `__restore` 中，真正从 S 态切换到 U 态的是：

```asm
sret
```

在它之前，`sstatus`、`sepc`、`sp` 和通用寄存器都已经恢复；`sret` 执行时：

- 把 `sepc` 写入 PC；
- 根据 `sstatus.SPP` 设置新的特权级；
- 根据 `sstatus.SPIE` 恢复 `sstatus.SIE`；
- 把 `SPIE` 置 1，并把 `SPP` 置为 U。

由于此时的 `sstatus.SPP` 是 U，`sret` 之后处理器从 `sepc` 处的用户指令
开始执行，因此进入 U 态。

### `__alltraps` L13：交换后 `sp` 和 `sscratch` 的含义

```asm
csrrw sp, sscratch, sp
```

进入 `__alltraps` 时，`sp` 是用户栈指针，`sscratch` 保存 kernel stack
指针。执行这条指令后：

- `sp` 变成 kernel stack 指针，后续可以在内核栈上分配并保存
  `TrapContext`；
- `sscratch` 变成用户栈指针，L34 再把它读出并保存到
  `TrapContext.x[2]`。

### 从 U 态进入 S 态的触发方式

对系统调用而言，U 态执行 `ecall`，硬件产生
Environment Call from U-mode 异常，然后跳转到 `stvec` 所指向的
`__alltraps`。因此 `ecall` 是主动发起该路径的指令；真正的特权级切换由
硬件在接受 trap 时完成。

更一般地，U 态的同步异常或中断也由硬件触发 trap 进入 S 态，此时不一定有
主动指令。例如定时器中断会在不修改用户 PC 的情况下触发 trap；非法指令和
非法内存访问则在执行出错指令时触发异常。进入 S 态后执行的第一段汇编就是
`__alltraps`，而 L13 是在硬件已经完成特权级切换之后执行的第一条指令。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾分别与 以下各位 就（与本次实验相关的）以下方面做过交流，还在代码中对应的位置以注释形式记录了具体的交流对象及内容：

deepseek-v4.1-flash 1017

2. 此外，我也参考了 以下资料 ，还在代码中对应的位置以注释形式记录了具体的参考来源及内容：

rCore-Tutotial-book-v3

3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。 我清楚地知道，从以上方面获得的信息在一定程度上降低了实验难度，可能会影响起评分。

4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。 我未曾也不会向他人（含此后各届同学）复制或公开我的实验代码，我有义务妥善保管好它们。 我提交至本实验的评测系统的代码，均无意于破坏或妨碍任何计算机系统的正常运转。 我清楚地知道，以上情况均为本课程纪律所禁止，若违反，对应的实验成绩将按“-100”分计。