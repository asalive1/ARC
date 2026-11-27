use std::{
    fs::{self, File},
    io::ErrorKind,
    os::unix::fs::MetadataExt,
    path::{Path, PathBuf},
};

use arc_application::{PortError, PortResult};
pub use arc_application::ProcessFenceIdentity;
use uuid::Uuid;

/// The lock file must be a persistent, host-local inode shared by every
/// instance of this worker identity. The host ID must be unique and uncloned.
/// Legacy workers that do not take this lock are not fenced by this type.
pub struct WorkerProcessFence {
    _lock: File,
    host_proc: PathBuf,
    identity: ProcessFenceIdentity,
}

impl WorkerProcessFence {
    pub fn acquire(lock_path: &Path, host_proc: &Path, host_id_path: &Path) -> PortResult<Self> {
        let host_proc = host_proc.canonicalize().map_err(|error| {
            fence_error("worker_fence_config_invalid", format!("host proc: {error}"))
        })?;
        validate_host_proc_mount(&host_proc)?;
        let host_id = read_uuid(host_id_path)?;
        let boot_id = read_uuid(&host_proc.join("sys/kernel/random/boot_id"))?;
        let proc_device = fs::metadata(&host_proc)
            .map_err(|error| {
                fence_error("worker_fence_config_invalid", format!("host proc: {error}"))
            })?
            .dev();
        let host_pid: u32 = fs::read_link(host_proc.join("self"))
            .map_err(|error| fence_error("worker_fence_config_invalid", error.to_string()))?
            .to_str()
            .ok_or_else(|| fence_error("worker_fence_config_invalid", "host PID is not UTF-8"))?
            .parse()
            .map_err(|error| {
                fence_error("worker_fence_config_invalid", format!("host PID: {error}"))
            })?;
        validate_pid_namespace(&host_proc, host_pid)?;
        let start_ticks = read_start_ticks(&host_proc, host_pid)?.ok_or_else(|| {
            fence_error("worker_fence_config_invalid", "current host PID is missing")
        })?;
        let lock = File::open(lock_path).map_err(|error| {
            fence_error("worker_fence_unavailable", format!("fence file: {error}"))
        })?;
        let metadata = lock.metadata().map_err(|error| {
            fence_error("worker_fence_unavailable", format!("fence file: {error}"))
        })?;
        if !metadata.is_file() {
            return Err(fence_error(
                "worker_fence_config_invalid",
                "fence file is not regular",
            ));
        }
        lock.try_lock().map_err(|error| {
            fence_error("worker_fence_unavailable", format!("fence lock: {error}"))
        })?;
        Ok(Self {
            _lock: lock,
            host_proc,
            identity: ProcessFenceIdentity {
                host_id,
                boot_id,
                device: metadata.dev(),
                inode: metadata.ino(),
                proc_device,
                host_pid,
                start_ticks,
            },
        })
    }

    pub fn identity(&self) -> &ProcessFenceIdentity {
        &self.identity
    }

    pub fn previous_process_terminated(&self, previous: &ProcessFenceIdentity) -> PortResult<bool> {
        if previous.host_id != self.identity.host_id
            || previous.device != self.identity.device
            || previous.inode != self.identity.inode
            || previous.host_pid == 0
        {
            return Err(fence_error(
                "worker_fence_domain_mismatch",
                "previous worker does not belong to this fence domain",
            ));
        }
        validate_host_proc_mount(&self.host_proc)?;
        let proc_device = fs::metadata(&self.host_proc)
            .map_err(|error| {
                fence_error("worker_fence_probe_failed", format!("host proc: {error}"))
            })?
            .dev();
        if proc_device != self.identity.proc_device
            || read_uuid(&self.host_proc.join("sys/kernel/random/boot_id"))?
                != self.identity.boot_id
        {
            return Err(fence_error(
                "worker_fence_domain_mismatch",
                "host proc changed while the fence was held",
            ));
        }
        if previous.boot_id != self.identity.boot_id {
            return Ok(true);
        }
        if previous.proc_device != self.identity.proc_device {
            return Err(fence_error(
                "worker_fence_domain_mismatch",
                "host PID namespace changed",
            ));
        }
        Ok(read_start_ticks(&self.host_proc, previous.host_pid)? != Some(previous.start_ticks))
    }
}

fn read_uuid(path: &Path) -> PortResult<Uuid> {
    let value = fs::read_to_string(path).map_err(|error| {
        fence_error(
            "worker_fence_config_invalid",
            format!("{}: {error}", path.display()),
        )
    })?;
    let id = Uuid::parse_str(value.trim()).map_err(|error| {
        fence_error(
            "worker_fence_config_invalid",
            format!("{}: {error}", path.display()),
        )
    })?;
    if id.is_nil() {
        return Err(fence_error(
            "worker_fence_config_invalid",
            "host identity is nil",
        ));
    }
    Ok(id)
}

fn validate_host_proc_mount(host_proc: &Path) -> PortResult<()> {
    let mount_path = host_proc
        .to_str()
        .filter(|path| !path.bytes().any(|b| b.is_ascii_whitespace() || b == b'\\'))
        .ok_or_else(|| {
            fence_error(
                "worker_fence_config_invalid",
                "host proc path cannot be represented in mountinfo",
            )
        })?;
    let mountinfo = fs::read_to_string("/proc/self/mountinfo").map_err(|error| {
        fence_error("worker_fence_config_invalid", format!("mountinfo: {error}"))
    })?;
    let matches: Vec<_> = mountinfo
        .lines()
        .filter_map(|line| check_proc_mount(line, mount_path))
        .collect();
    if matches.as_slice() != [true] {
        return Err(fence_error(
            "worker_fence_config_invalid",
            "host proc must be a root procfs mount without hidepid restrictions",
        ));
    }
    Ok(())
}

fn check_proc_mount(line: &str, mount_path: &str) -> Option<bool> {
    let (before, after) = line.split_once(" - ")?;
    let fields: Vec<_> = before.split_whitespace().collect();
    let after: Vec<_> = after.split_whitespace().collect();
    if fields.get(4) != Some(&mount_path) {
        return None;
    }
    Some(
        fields.get(3) == Some(&"/")
            && after.first() == Some(&"proc")
            && !fields.iter().chain(&after).any(|field| {
                field
                    .split(',')
                    .any(|option| option.starts_with("hidepid=") && option != "hidepid=0")
            }),
    )
}

fn validate_pid_namespace(host_proc: &Path, host_pid: u32) -> PortResult<()> {
    let status = fs::read_to_string(host_proc.join("self/status")).map_err(|error| {
        fence_error(
            "worker_fence_config_invalid",
            format!("host process status: {error}"),
        )
    })?;
    let line = status
        .lines()
        .find_map(|line| line.strip_prefix("NSpid:"))
        .ok_or_else(|| {
            fence_error(
                "worker_fence_config_invalid",
                "host PID namespace is unavailable",
            )
        })?;
    let pids: Result<Vec<u32>, _> = line.split_whitespace().map(str::parse).collect();
    let pids = pids.map_err(|error| {
        fence_error(
            "worker_fence_config_invalid",
            format!("host PID namespace: {error}"),
        )
    })?;
    if pids.len() < 2 || pids.first() != Some(&host_pid) || pids.last() != Some(&std::process::id())
    {
        return Err(fence_error(
            "worker_fence_config_invalid",
            "host proc does not expose the worker's outer PID namespace",
        ));
    }
    Ok(())
}

fn read_start_ticks(host_proc: &Path, host_pid: u32) -> PortResult<Option<u64>> {
    let path = host_proc.join(host_pid.to_string()).join("stat");
    let stat = match fs::read_to_string(&path) {
        Ok(stat) => stat,
        Err(error) if error.kind() == ErrorKind::NotFound => return Ok(None),
        Err(error) => {
            return Err(fence_error(
                "worker_fence_probe_failed",
                format!("process stat: {error}"),
            ))
        }
    };
    let fields = stat
        .rsplit_once(')')
        .ok_or_else(|| fence_error("worker_fence_probe_failed", "process stat is malformed"))?
        .1;
    let ticks = fields
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| fence_error("worker_fence_probe_failed", "process start time is missing"))?
        .parse()
        .map_err(|error| {
            fence_error(
                "worker_fence_probe_failed",
                format!("process start time: {error}"),
            )
        })?;
    Ok(Some(ticks))
}

fn fence_error(code: &'static str, message: impl Into<String>) -> PortError {
    PortError {
        code,
        message: message.into(),
    }
}

#[cfg(test)]
mod tests {
    use std::{
        io::{BufRead, BufReader},
        process::{Command, Stdio},
        time::Duration,
    };

    use super::*;

    #[test]
    fn refuses_proc_with_hidden_processes() {
        let visible = "42 1 0:42 / /hostproc ro,relatime - proc proc rw,hidepid=0";
        let hidden_mount = "42 1 0:42 / /hostproc ro,hidepid=2 - proc proc rw";
        let hidden_superblock = "42 1 0:42 / /hostproc ro - proc proc rw,hidepid=invisible";
        assert_eq!(check_proc_mount(visible, "/hostproc"), Some(true));
        assert_eq!(check_proc_mount(hidden_mount, "/hostproc"), Some(false));
        assert_eq!(
            check_proc_mount(hidden_superblock, "/hostproc"),
            Some(false)
        );
    }

    #[test]
    fn refuses_proc_without_outer_pid_visibility() {
        let directory = std::env::temp_dir();
        let lock_path = directory.join(format!("arc-fence-{}", Uuid::new_v4()));
        let host_id_path = directory.join(format!("arc-host-{}", Uuid::new_v4()));
        fs::write(&lock_path, b"").unwrap();
        fs::write(&host_id_path, Uuid::new_v4().to_string()).unwrap();
        let error = WorkerProcessFence::acquire(&lock_path, Path::new("/proc"), &host_id_path)
            .err()
            .expect("container-local proc is not a host process fence");
        assert_eq!(error.code, "worker_fence_config_invalid");
        fs::remove_file(lock_path).unwrap();
        fs::remove_file(host_id_path).unwrap();
    }

    #[test]
    #[ignore = "requires a host proc mount at ARC_TEST_HOST_PROC"]
    fn process_fence_child() {
        let host_proc = std::env::var("ARC_TEST_HOST_PROC").unwrap();
        let lock_path = std::env::var("ARC_TEST_FENCE_LOCK").unwrap();
        let host_id_path = std::env::var("ARC_TEST_FENCE_HOST_ID").unwrap();
        let fence = WorkerProcessFence::acquire(
            Path::new(&lock_path),
            Path::new(&host_proc),
            Path::new(&host_id_path),
        )
        .unwrap();
        println!(
            "FENCE_READY {} {}",
            fence.identity.host_pid, fence.identity.start_ticks
        );
        std::thread::sleep(Duration::from_secs(60));
    }

    #[test]
    #[ignore = "requires a host proc mount at ARC_TEST_HOST_PROC"]
    fn process_fence_blocks_duplicates_and_checks_process_exit() {
        let host_proc = std::env::var("ARC_TEST_HOST_PROC").unwrap();
        let directory = std::env::temp_dir();
        let lock_path = directory.join(format!("arc-fence-{}", Uuid::new_v4()));
        let host_id_path = directory.join(format!("arc-host-{}", Uuid::new_v4()));
        fs::write(&lock_path, b"").unwrap();
        fs::write(&host_id_path, Uuid::new_v4().to_string()).unwrap();
        let mut child = Command::new(std::env::current_exe().unwrap())
            .args([
                "--exact",
                "process_fence::tests::process_fence_child",
                "--ignored",
                "--nocapture",
            ])
            .env("ARC_TEST_HOST_PROC", &host_proc)
            .env("ARC_TEST_FENCE_LOCK", &lock_path)
            .env("ARC_TEST_FENCE_HOST_ID", &host_id_path)
            .stdout(Stdio::piped())
            .spawn()
            .unwrap();
        let mut reader = BufReader::new(child.stdout.take().unwrap());
        let ready = loop {
            let mut line = String::new();
            assert_ne!(
                reader.read_line(&mut line).unwrap(),
                0,
                "fence child ended before acquiring the lock"
            );
            if let Some(position) = line.find("FENCE_READY ") {
                break line[position..].to_owned();
            }
        };
        let parts: Vec<_> = ready.split_whitespace().collect();
        let host_pid: u32 = parts[1].parse().unwrap();
        let start_ticks: u64 = parts[2].parse().unwrap();
        assert_eq!(
            WorkerProcessFence::acquire(&lock_path, Path::new(&host_proc), &host_id_path)
                .err()
                .unwrap()
                .code,
            "worker_fence_unavailable"
        );
        let other_lock = directory.join(format!("arc-fence-{}", Uuid::new_v4()));
        fs::write(&other_lock, b"").unwrap();
        let observer =
            WorkerProcessFence::acquire(&other_lock, Path::new(&host_proc), &host_id_path).unwrap();
        let mut live = observer.identity().clone();
        live.host_pid = host_pid;
        live.start_ticks = start_ticks;
        assert!(!observer.previous_process_terminated(&live).unwrap());
        live.start_ticks = live.start_ticks.checked_add(1).unwrap();
        assert!(observer.previous_process_terminated(&live).unwrap());
        drop(observer);
        fs::remove_file(other_lock).unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        let fence =
            WorkerProcessFence::acquire(&lock_path, Path::new(&host_proc), &host_id_path).unwrap();
        let mut previous = fence.identity().clone();
        previous.host_pid = host_pid;
        previous.start_ticks = start_ticks;
        assert!(fence.previous_process_terminated(&previous).unwrap());
        assert!(!fence.previous_process_terminated(fence.identity()).unwrap());
        previous.inode ^= 1;
        assert_eq!(
            fence
                .previous_process_terminated(&previous)
                .unwrap_err()
                .code,
            "worker_fence_domain_mismatch"
        );
        drop(fence);
        fs::remove_file(lock_path).unwrap();
        fs::remove_file(host_id_path).unwrap();
    }
}
