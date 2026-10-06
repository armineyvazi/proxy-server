//! Read/write operations on the shared log ring buffer.
//!
//! `drain_logs` returns exactly the entries written since the previous drain,
//! with no duplicates under concurrent callers, by holding the mutex across
//! the whole drain (read + advance `drain_pos`).

use crate::ipc::{LogBuffer, LogEntry, MAX_LOG_ENTRIES};
use std::ptr;

/// Append one entry for a completed request.
///
/// # Safety
/// `buf` must point to an initialized `LogBuffer` in the inherited shared mapping.
pub unsafe fn log_request(buf: *mut LogBuffer, ip: &str, pid: u32) {
    // SAFETY: `time(NULL)` is always valid; reading the clock outside the lock
    // keeps the critical section minimal.
    let timestamp = unsafe { libc::time(ptr::null_mut()) as u64 };
    let entry = make_entry(timestamp, ip, pid);

    // SAFETY: the mutex and ring live in the shared region; a full ring
    // overwrites the oldest unread entry rather than blocking the proxy.
    unsafe {
        libc::pthread_mutex_lock(ptr::addr_of_mut!((*buf).mutex));
        let slot = ((*buf).write_pos as usize) % MAX_LOG_ENTRIES;
        (*buf).entries[slot] = entry;
        (*buf).write_pos = (*buf).write_pos.wrapping_add(1);
        libc::pthread_mutex_unlock(ptr::addr_of_mut!((*buf).mutex));
    }
}

/// Drain every entry written since the last call, formatted as
/// `"<timestamp> <client_ip> <pid>"`.
///
/// # Safety
/// Same requirements as [`log_request`].
pub unsafe fn drain_logs(buf: *mut LogBuffer) -> Vec<String> {
    // SAFETY: all shared-state access happens under the lock.
    unsafe {
        libc::pthread_mutex_lock(ptr::addr_of_mut!((*buf).mutex));

        let to = (*buf).write_pos as usize;
        let from = (*buf).drain_pos as usize;
        let count = (to - from).min(MAX_LOG_ENTRIES);
        let start = to - count;

        let lines: Vec<String> = (0..count)
            .map(|i| format_entry(&(*buf).entries[(start + i) % MAX_LOG_ENTRIES]))
            .collect();

        (*buf).drain_pos = (*buf).write_pos;
        libc::pthread_mutex_unlock(ptr::addr_of_mut!((*buf).mutex));
        lines
    }
}

fn make_entry(timestamp: u64, ip: &str, pid: u32) -> LogEntry {
    let mut client_ip = [0u8; 16];
    let bytes = ip.as_bytes();
    let len = bytes.len().min(15);
    client_ip[..len].copy_from_slice(&bytes[..len]);
    LogEntry {
        timestamp,
        client_ip,
        pid,
        valid: 1,
    }
}

fn format_entry(e: &LogEntry) -> String {
    let null_pos = e.client_ip.iter().position(|&b| b == 0).unwrap_or(16);
    let ip = std::str::from_utf8(&e.client_ip[..null_pos]).unwrap_or("unknown");
    format!("{} {} {}", e.timestamp, ip, e.pid)
}
