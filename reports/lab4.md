# lab4 实验报告：文件系统与文件系统调用

## 1. 本实验完成的内容

本章在 easy-fs 之上补齐了文件元数据和硬链接相关的系统调用：

- `sys_fstat`：获取已打开文件的 `dev`、`ino`、`mode`、`nlink`；
- `sys_linkat`：为已有文件创建一个新的目录项，并增加硬链接计数；
- `sys_unlinkat`：删除目录项并减少硬链接计数；
- easy-fs 的 `DiskInode` 增加 `nlink` 字段，`Inode` 增加 `inode_id`、`is_dir`、`nlink`、`link`、`unlink`；
- 内核的 `File` trait 增加 `stat` 方法，`OSInode` 负责把 easy-fs 的元数据转换成 `Stat`；
- 同时保留 ch4/ch5 的 `sys_get_time`、`mmap`、`munmap`、`spawn`、`set_priority`，使累积测例继续通过。

主要修改文件：

- `easy-fs/src/layout.rs`：`DiskInode::nlink` 及增减方法；
- `easy-fs/src/efs.rs`：由块位置反推 `inode_id`；
- `easy-fs/src/vfs.rs`：`Inode::link`、`Inode::unlink`、`Inode::nlink`、`Inode::is_dir`；
- `os/src/fs/mod.rs`：`File::stat`、`Stat::new`；
- `os/src/fs/inode.rs`：`OSInode::stat`；
- `os/src/syscall/fs.rs`：三个文件系统调用。

## 2. 为 easy-fs 增加硬链接

原模板的 `DiskInode` 只有 `size`、`direct`、`indirect1`、`indirect2` 和类型，没有链接计数，因此无法实现 `fstat` 的 `nlink`。本实验在 `DiskInode` 末尾增加 `nlink: u32`，初始化时置 1。

`Inode` 本身只保存 `block_id` 和 `block_offset`，没有保存 inode 编号。`fstat` 需要返回 `ino`，因此新增：

```rust
pub fn get_inode_id(&self, block_id: u32, block_offset: usize) -> u32 {
    let inode_size = core::mem::size_of::<DiskInode>();
    let inodes_per_block = (BLOCK_SZ / inode_size) as u32;
    (block_id - self.inode_area_start_block) * inodes_per_block
        + (block_offset / inode_size) as u32
}
```

`DiskInode` 的大小发生了变化，inode 区的布局也随之变化。由于 easy-fs-fuse 和内核使用同一份 easy-fs 源码，每次运行都会重新生成 `fs.img`，两边保持一致。

### `link`

`ROOT_INODE.link(old_name, new_name)` 的流程：

1. 在根目录中查找 `old_name`，得到目标 inode id；若不存在返回失败；
2. 若 `new_name` 已存在，返回失败；
3. 在根目录末尾追加一个 `DirEntry`，其 `inode_id` 指向旧 inode；
4. 增加目标 inode 的 `nlink`；
5. `block_cache_sync_all()` 写回磁盘。

### `unlink`

1. 在根目录中查找目标目录项，记录其下标和 inode id；
2. 用最后一个目录项覆盖被删除项，再把目录大小减一个 `DIRENT_SZ`；
3. 减少目标 inode 的 `nlink`；
4. 同步块缓存。

题目说明允许 unlink 后不回收 inode 和数据块，因此实现只减少链接计数、删除目录项，不做 inode 和数据块的回收。

## 3. `fstat`

内核的 `fd_table` 中保存的是 `Arc<dyn File>`，无法直接向下转型。为了保持接口简单，在 `File` trait 中增加默认方法：

```rust
fn stat(&self) -> Option<Stat> {
    None
}
```

`OSInode` 重写该方法，读取底层 `easy_fs::Inode` 的类型、inode 编号和链接计数：

```rust
let mode = if inode.is_dir() { StatMode::DIR } else { StatMode::FILE };
Some(Stat::new(0, inode.inode_id() as u64, mode, inode.nlink()))
```

`sys_fstat` 从当前进程的 `fd_table` 取出文件，调用 `stat()`，再用 `translated_byte_buffer` 把 `Stat` 按字节写回用户指针。`Stdin`/`Stdout` 使用默认的 `None`，对它们调用 `fstat` 返回 `-1`。

## 4. 测试结果

使用官方检查器：

```sh
make -C ci-user test CHAPTER=6
```

测试汇总为 `Test passed: 31/31`，其中文件系统相关输出为：

```text
file_test passed!
Test file0 OK!
Test fstat OK!
Test link OK!
Test mass open/unlink OK!
```

`ch6_file2` 依次创建 3 个硬链接并检查 `nlink` 从 1 增至 4；unlink 原名后仍能通过 linkname0 读到相同内容，且 `ino`、`dev` 与原来一致；这说明目录项和 inode 的分离是正确的。

## 5. 实验分析

硬链接的本质是“多个目录项指向同一个 inode”。因此 `link` 不能复制文件数据，只能在目录中新增一个指向旧 inode 的 `DirEntry`，并增加 `nlink`；`unlink` 也不能直接删除 inode，而要先把链接计数减一，只有当计数为 0 且没有打开的文件引用时才应真正回收。本实验为简化没有实现回收，但目录项和 `nlink` 的变化已经符合测例要求。

`fstat` 体现了 VFS 抽象的一个常见问题：文件描述符表统一保存 `dyn File`，但元数据只有文件系统 inode 才有。通过 trait 默认方法返回 `Option<Stat>`，既避免了向下转型，也让标准输入输出自然地返回“不支持”。

## 荣誉准则

1. 在完成本次实验的过程（含此前学习的过程）中，我曾与 deepseek-v4.1-flash 就实验实现与调试进行交流，并在代码对应位置记录了参考来源。
2. 此外，我也参考了 rCore-Tutorial-Book-v3、rCore-Tutorial-Guide 以及 easy-fs 的源码。
3. 我独立完成了本次实验除以上方面之外的所有工作，包括代码与文档。
4. 我从未使用过他人的代码，不管是原封不动地复制，还是经过了某些等价转换。
