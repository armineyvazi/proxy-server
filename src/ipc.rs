//! Cross-process shared memory: an mmap'd ring buffer guarded by a
//! process-shared pthread mutex.
//!
//! After `fork()` each child has a private address space, so shared state uses
//! `mmap(MAP_SHARED | MAP_ANON)` with a `pthread_mutex_t` that has
//! `PTHREAD_PROCESS_SHARED` set.

use std::{mem, ptr};

/// Ring buffer capacity. 65536 × 32 bytes = 2 MB.
pub const MAX_LOG_ENTRIES: usize = 65536;

/// One proxied-request log entry in shared memory.
#[repr(C)]
#[derive(Copy, Clone)]
pub struct LogEntry {
    pub timestamp: u64,
    pub client_ip: [u8; 16], // null-padded ASCII IPv4
    pub pid: u32,
    pub valid: u8,
}

/// Shared log buffer living in the mmap region.
///
/// `write_pos` / `drain_pos` are monotonic counters (slot = pos % capacity).
/// Every field after `mutex` may only be touched while holding `mutex`.
#[repr(C)]
pub struct LogBuffer {
    pub mutex: libc::pthread_mutex_t,
    pub write_pos: u64,
    pub drain_pos: u64,
    pub entries: [LogEntry; MAX_LOG_ENTRIES],
}

/// Allocate, zero, and initialize the shared `LogBuffer`.
///
/// Must be called by the supervisor *before* any `fork()`.
///
/// # Safety
/// The pointer is valid for the lifetime of the process group; the caller must
/// not unmap it while any worker is running.
pub unsafe fn allocate_shared_region() -> *mut LogBuffer {
    let size = mem::size_of::<LogBuffer>();

    // SAFETY: null hint lets the kernel choose the address; MAP_SHARED|MAP_ANON
    // with fd=-1 is the documented anonymous-shared-memory call.
    let ptr = unsafe {
        libc::mmap(
            ptr::null_mut(),
            size,
            libc::PROT_READ | libc::PROT_WRITE,
            libc::MAP_SHARED | libc::MAP_ANON,
            -1,
            0,
        )
    };

    if ptr == libc::MAP_FAILED {
        eprintln!(
            "fatal: mmap failed ({}): {}",
            size,
            std::io::Error::last_os_error()
        );
        std::process::exit(1);
    }

    let buf = ptr as *mut LogBuffer;

    // SAFETY: `buf` points to freshly mapped writable memory. `addr_of_mut!`
    // avoids forming a `&mut` to memory other processes will read concurrently.
    unsafe {
        ptr::write_bytes(buf, 0, 1);
        init_shared_mutex(ptr::addr_of_mut!((*buf).mutex));
    }

    buf
}

/// Initialize a `pthread_mutex_t` with `PTHREAD_PROCESS_SHARED`.
///
/// # Safety
/// `mutex` must point into the shared region and be initialized exactly once.
unsafe fn init_shared_mutex(mutex: *mut libc::pthread_mutex_t) {
    // SAFETY: the attr struct is plain data; each pthread call is checked.
    unsafe {
        let mut attr: libc::pthread_mutexattr_t = mem::zeroed();
        assert_eq!(
            libc::pthread_mutexattr_init(&mut attr),
            0,
            "pthread_mutexattr_init"
        );
        assert_eq!(
            libc::pthread_mutexattr_setpshared(&mut attr, libc::PTHREAD_PROCESS_SHARED),
            0,
            "pthread_mutexattr_setpshared"
        );
        assert_eq!(
            libc::pthread_mutex_init(mutex, &attr),
            0,
            "pthread_mutex_init"
        );
        assert_eq!(
            libc::pthread_mutexattr_destroy(&mut attr),
            0,
            "pthread_mutexattr_destroy"
        );
    }
}
