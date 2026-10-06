//! Supervisor: parse args, size the worker pool, allocate the shared log
//! buffer, fork one worker per core, and respawn workers that exit.

mod ipc;
mod logs;
mod proxy;
mod worker;

use ipc::LogBuffer;
use std::collections::HashMap;

fn main() {
    let (inbound, outbound) = parse_args();

    // Raise the FD limit before forking so workers inherit it.
    raise_fd_limit();

    // Allocate before fork() so every child inherits the same MAP_SHARED mapping.
    let log_buf: *mut LogBuffer = unsafe { ipc::allocate_shared_region() };

    let num_cpus = worker_count();
    eprintln!(
        "proxy-server: inbound={} outbound={} workers={}",
        inbound, outbound, num_cpus
    );

    let mut workers: HashMap<libc::pid_t, usize> = HashMap::new();
    for cpu_id in 0..num_cpus {
        let pid = spawn_worker(cpu_id, &inbound, &outbound, log_buf);
        workers.insert(pid, cpu_id);
        eprintln!("proxy-server: spawned worker pid={} cpu={}", pid, cpu_id);
    }

    supervise_loop(workers, &inbound, &outbound, log_buf);
}

/// Fork one worker. In the child this never returns; in the parent it returns
/// the child PID.
fn spawn_worker(
    cpu_id: usize,
    inbound: &str,
    outbound: &str,
    log_buf: *mut LogBuffer,
) -> libc::pid_t {
    // SAFETY: the child only runs async-signal-safe work before entering its
    // own event loop and never returns to this function.
    let pid = unsafe { libc::fork() };
    match pid {
        -1 => {
            eprintln!("fatal: fork() failed: {}", std::io::Error::last_os_error());
            std::process::exit(1);
        }
        0 => worker::run_worker(cpu_id, inbound, outbound, log_buf),
        child_pid => child_pid,
    }
}

/// Poll for exited workers and respawn each on the CPU it vacated.
fn supervise_loop(
    mut workers: HashMap<libc::pid_t, usize>,
    inbound: &str,
    outbound: &str,
    log_buf: *mut LogBuffer,
) -> ! {
    loop {
        // SAFETY: waitpid with WNOHANG is non-blocking and writes only `status`.
        let (exited_pid, status) = unsafe {
            let mut status: libc::c_int = 0;
            let pid = libc::waitpid(-1, &mut status, libc::WNOHANG);
            (pid, status)
        };

        if exited_pid > 0 {
            if libc::WIFSIGNALED(status) {
                eprintln!(
                    "proxy-server: worker pid={} killed by signal {}",
                    exited_pid,
                    libc::WTERMSIG(status)
                );
            } else if libc::WIFEXITED(status) {
                eprintln!(
                    "proxy-server: worker pid={} exited with code {}",
                    exited_pid,
                    libc::WEXITSTATUS(status)
                );
            }
            if let Some(cpu_id) = workers.remove(&exited_pid) {
                let new_pid = spawn_worker(cpu_id, inbound, outbound, log_buf);
                workers.insert(new_pid, cpu_id);
                eprintln!(
                    "proxy-server: restarted worker cpu={} new_pid={}",
                    cpu_id, new_pid
                );
            }
        }

        // 100 ms poll keeps crash-detection latency under the 1 s target.
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
}

/// Number of workers: one per logical CPU, or `PROXY_WORKERS` when set.
fn worker_count() -> usize {
    match std::env::var("PROXY_WORKERS")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
    {
        Some(n) if n >= 1 => n,
        _ => count_cpus(),
    }
}

fn count_cpus() -> usize {
    let n = unsafe { libc::sysconf(libc::_SC_NPROCESSORS_ONLN) };
    if n <= 0 {
        eprintln!("warning: sysconf(_SC_NPROCESSORS_ONLN) failed, defaulting to 1");
        1
    } else {
        n as usize
    }
}

/// Raise `RLIMIT_NOFILE` toward 1M (capped at the hard limit) before forking.
fn raise_fd_limit() {
    // SAFETY: get/setrlimit operate on a plain C struct with no Rust invariants.
    unsafe {
        let mut rlim: libc::rlimit = std::mem::zeroed();
        if libc::getrlimit(libc::RLIMIT_NOFILE, &mut rlim) != 0 {
            eprintln!(
                "warning: getrlimit(RLIMIT_NOFILE) failed: {}",
                std::io::Error::last_os_error()
            );
            return;
        }
        let target: u64 = 1_048_576;
        rlim.rlim_cur = if rlim.rlim_max == libc::RLIM_INFINITY {
            target
        } else {
            target.min(rlim.rlim_max)
        };
        if libc::setrlimit(libc::RLIMIT_NOFILE, &rlim) == 0 {
            eprintln!("proxy-server: fd limit raised to {}", rlim.rlim_cur);
        } else {
            eprintln!(
                "warning: setrlimit(RLIMIT_NOFILE, {}) failed: {} — run with higher Docker ulimits",
                rlim.rlim_cur,
                std::io::Error::last_os_error()
            );
        }
    }
}

fn parse_args() -> (String, String) {
    let args: Vec<String> = std::env::args().collect();
    let mut inbound = None;
    let mut outbound = None;

    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--inbound" => {
                i += 1;
                inbound = args.get(i).cloned();
            }
            "--outbound" => {
                i += 1;
                outbound = args.get(i).cloned();
            }
            other => eprintln!("unknown argument: {}", other),
        }
        i += 1;
    }

    match (inbound, outbound) {
        (Some(i), Some(o)) => (i, o),
        _ => {
            eprintln!("usage: proxy-server --inbound <addr:port> --outbound <host[:port]>");
            std::process::exit(1);
        }
    }
}
