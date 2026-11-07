//! Process management syscalls
use crate::mm::{translated_byte_buffer, MapPermission, PageTable, VirtAddr};
use crate::task::{
    change_program_brk, current_syscall_count, current_user_token, exit_current_and_run_next,
    mmap_current, munmap_current, suspend_current_and_run_next,
};
use crate::timer::get_time_us;
use core::mem::size_of;
use core::slice;

#[repr(C)]
#[derive(Debug)]
pub struct TimeVal {
    pub sec: usize,
    pub usec: usize,
}

/// task exits and submit an exit code
pub fn sys_exit(_exit_code: i32) -> ! {
    trace!("kernel: sys_exit");
    exit_current_and_run_next();
    panic!("Unreachable in sys_exit!");
}

/// current task gives up resources for other tasks
pub fn sys_yield() -> isize {
    trace!("kernel: sys_yield");
    suspend_current_and_run_next();
    0
}

/// Read one byte from user space through the current page table.
fn read_user_byte(token: usize, addr: usize) -> Option<u8> {
    // SV39 user space only occupies the lower half; reject non-canonical addresses
    if addr >= (1usize << 38) {
        return None;
    }
    let va = VirtAddr::from(addr);
    let pte = PageTable::from_token(token).translate(va.floor())?;
    if !pte.is_valid() || !pte.readable() {
        return None;
    }
    Some(pte.ppn().get_bytes_array()[va.page_offset()])
}

/// Write one byte to user space through the current page table.
fn write_user_byte(token: usize, addr: usize, data: u8) -> bool {
    if addr >= (1usize << 38) {
        return false;
    }
    let va = VirtAddr::from(addr);
    if let Some(pte) = PageTable::from_token(token).translate(va.floor()) {
        if pte.is_valid() && pte.writable() {
            pte.ppn().get_bytes_array()[va.page_offset()] = data;
            return true;
        }
    }
    false
}

/// get time with second and microsecond
pub fn sys_get_time(ts: *mut TimeVal, _tz: usize) -> isize {
    trace!("kernel: sys_get_time");
    let token = current_user_token();
    let time_us = get_time_us();
    let time_val = TimeVal {
        sec: time_us / 1_000_000,
        usec: time_us % 1_000_000,
    };
    let bytes = unsafe {
        slice::from_raw_parts(
            &time_val as *const TimeVal as *const u8,
            size_of::<TimeVal>(),
        )
    };
    let mut copied = 0;
    for buffer in translated_byte_buffer(token, ts as *const u8, size_of::<TimeVal>()) {
        let len = buffer.len().min(bytes.len() - copied);
        buffer[..len].copy_from_slice(&bytes[copied..copied + len]);
        copied += len;
    }
    0
}

/// trace user memory or syscall counts
pub fn sys_trace(trace_request: usize, id: usize, data: usize) -> isize {
    trace!("kernel: sys_trace");
    let token = current_user_token();
    match trace_request {
        // read one byte at user address id
        0 => read_user_byte(token, id).map_or(-1, |byte| byte as isize),
        // write the lowest byte of data to user address id
        1 => {
            if write_user_byte(token, id, data as u8) {
                0
            } else {
                -1
            }
        }
        // query how many times syscall id has been called
        2 => current_syscall_count(id) as isize,
        _ => -1,
    }
}

/// map a user memory area
pub fn sys_mmap(start: usize, len: usize, port: usize) -> isize {
    trace!("kernel: sys_mmap");
    if port == 0 || port > 0b111 {
        return -1;
    }
    let mut permission = MapPermission::U;
    if port & 0b001 != 0 {
        permission |= MapPermission::R;
    }
    if port & 0b010 != 0 {
        permission |= MapPermission::W;
    }
    if port & 0b100 != 0 {
        permission |= MapPermission::X;
    }
    if mmap_current(start, len, permission) {
        0
    } else {
        -1
    }
}

/// unmap a user memory area
pub fn sys_munmap(start: usize, len: usize) -> isize {
    trace!("kernel: sys_munmap");
    if munmap_current(start, len) {
        0
    } else {
        -1
    }
}
/// change data segment size
pub fn sys_sbrk(size: i32) -> isize {
    trace!("kernel: sys_sbrk");
    if let Some(old_brk) = change_program_brk(size) {
        old_brk as isize
    } else {
        -1
    }
}
