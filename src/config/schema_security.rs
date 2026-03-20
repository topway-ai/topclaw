use super::{
    default_true, AuditConfig, EstopConfig, OtpConfig, ResourceLimitsConfig, SandboxConfig,
    SecurityConfig, SyscallAnomalyConfig,
};

pub(crate) fn default_semantic_guard_collection() -> String {
    "semantic_guard".to_string()
}

pub(crate) fn default_semantic_guard_threshold() -> f64 {
    0.82
}

pub(crate) fn default_syscall_anomaly_max_denied_events_per_minute() -> u32 {
    5
}

pub(crate) fn default_syscall_anomaly_max_total_events_per_minute() -> u32 {
    120
}

pub(crate) fn default_syscall_anomaly_max_alerts_per_minute() -> u32 {
    30
}

pub(crate) fn default_syscall_anomaly_alert_cooldown_secs() -> u64 {
    20
}

pub(crate) fn default_syscall_anomaly_log_path() -> String {
    "syscall-anomalies.log".to_string()
}

pub(crate) fn default_syscall_anomaly_baseline_syscalls() -> Vec<String> {
    vec![
        "read".to_string(),
        "write".to_string(),
        "open".to_string(),
        "openat".to_string(),
        "close".to_string(),
        "stat".to_string(),
        "fstat".to_string(),
        "newfstatat".to_string(),
        "lseek".to_string(),
        "mmap".to_string(),
        "mprotect".to_string(),
        "munmap".to_string(),
        "brk".to_string(),
        "rt_sigaction".to_string(),
        "rt_sigprocmask".to_string(),
        "ioctl".to_string(),
        "fcntl".to_string(),
        "access".to_string(),
        "pipe2".to_string(),
        "dup".to_string(),
        "dup2".to_string(),
        "dup3".to_string(),
        "epoll_create1".to_string(),
        "epoll_ctl".to_string(),
        "epoll_wait".to_string(),
        "poll".to_string(),
        "ppoll".to_string(),
        "select".to_string(),
        "futex".to_string(),
        "clock_gettime".to_string(),
        "nanosleep".to_string(),
        "getpid".to_string(),
        "gettid".to_string(),
        "set_tid_address".to_string(),
        "set_robust_list".to_string(),
        "clone".to_string(),
        "clone3".to_string(),
        "fork".to_string(),
        "execve".to_string(),
        "wait4".to_string(),
        "exit".to_string(),
        "exit_group".to_string(),
        "socket".to_string(),
        "connect".to_string(),
        "accept".to_string(),
        "accept4".to_string(),
        "listen".to_string(),
        "sendto".to_string(),
        "recvfrom".to_string(),
        "sendmsg".to_string(),
        "recvmsg".to_string(),
        "getsockname".to_string(),
        "getpeername".to_string(),
        "setsockopt".to_string(),
        "getsockopt".to_string(),
        "getrandom".to_string(),
        "statx".to_string(),
    ]
}

impl Default for SecurityConfig {
    fn default() -> Self {
        Self {
            sandbox: SandboxConfig::default(),
            resources: ResourceLimitsConfig::default(),
            audit: AuditConfig::default(),
            otp: OtpConfig::default(),
            estop: EstopConfig::default(),
            syscall_anomaly: SyscallAnomalyConfig::default(),
            canary_tokens: default_true(),
            semantic_guard: false,
            semantic_guard_collection: default_semantic_guard_collection(),
            semantic_guard_threshold: default_semantic_guard_threshold(),
        }
    }
}

impl Default for SyscallAnomalyConfig {
    fn default() -> Self {
        Self {
            enabled: default_true(),
            strict_mode: false,
            alert_on_unknown_syscall: default_true(),
            max_denied_events_per_minute: default_syscall_anomaly_max_denied_events_per_minute(),
            max_total_events_per_minute: default_syscall_anomaly_max_total_events_per_minute(),
            max_alerts_per_minute: default_syscall_anomaly_max_alerts_per_minute(),
            alert_cooldown_secs: default_syscall_anomaly_alert_cooldown_secs(),
            log_path: default_syscall_anomaly_log_path(),
            baseline_syscalls: default_syscall_anomaly_baseline_syscalls(),
        }
    }
}
