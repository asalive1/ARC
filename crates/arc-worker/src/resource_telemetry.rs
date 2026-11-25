use std::{io, time::Duration};

#[cfg(target_os = "linux")]
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    time::Instant,
};

use sysinfo::System;

pub(crate) const CPU_SAMPLE_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub(crate) struct ResourceUsage {
    pub(crate) cpu_utilization_ratio: Option<f32>,
    pub(crate) memory_utilization_ratio: Option<f32>,
}

pub(crate) struct ResourceTelemetry {
    system: System,
    #[cfg(target_os = "linux")]
    cgroups: Option<CgroupTelemetry>,
}

impl ResourceTelemetry {
    pub(crate) fn new() -> io::Result<Self> {
        let mut system = System::new();
        system.refresh_cpu_all();
        system.refresh_memory();

        #[cfg(target_os = "linux")]
        let cgroups = CgroupTelemetry::discover()?;

        #[cfg(target_os = "linux")]
        let mut telemetry = Self { system, cgroups };
        #[cfg(not(target_os = "linux"))]
        let telemetry = Self { system };

        #[cfg(target_os = "linux")]
        if let Some(cgroups) = telemetry.cgroups.as_mut() {
            cgroups.prime_cpu_counters()?;
        }

        Ok(telemetry)
    }

    pub(crate) fn sample(&mut self) -> io::Result<ResourceUsage> {
        self.system.refresh_cpu_all();
        self.system.refresh_memory();

        let host_cpu_ratio = percentage_to_ratio(self.system.global_cpu_usage());
        let host_memory_ratio = usage_ratio(self.system.used_memory(), self.system.total_memory());

        #[cfg(target_os = "linux")]
        if let Some(cgroups) = self.cgroups.as_mut() {
            let cpu = cgroups.sample_cpu_ratio()?.or(host_cpu_ratio);
            let memory = cgroups.sample_memory_ratio()?.or(host_memory_ratio);
            return Ok(ResourceUsage {
                cpu_utilization_ratio: cpu,
                memory_utilization_ratio: memory,
            });
        }

        Ok(ResourceUsage {
            cpu_utilization_ratio: host_cpu_ratio,
            memory_utilization_ratio: host_memory_ratio,
        })
    }
}

fn percentage_to_ratio(percent: f32) -> Option<f32> {
    percent
        .is_finite()
        .then(|| (percent / 100.0).clamp(0.0, 1.0))
}

fn usage_ratio(used: u64, total: u64) -> Option<f32> {
    (total > 0).then(|| (used as f64 / total as f64).clamp(0.0, 1.0) as f32)
}

fn cpu_usage_ratio(usage_delta_micros: u64, elapsed: Duration, quota_cores: f64) -> Option<f32> {
    let elapsed_micros = elapsed.as_secs_f64() * 1_000_000.0;
    if elapsed_micros <= 0.0 || quota_cores <= 0.0 {
        return None;
    }
    Some((usage_delta_micros as f64 / elapsed_micros / quota_cores).clamp(0.0, 1.0) as f32)
}

#[cfg(target_os = "linux")]
struct CgroupTelemetry {
    kind: CgroupKind,
    previous_cpu: HashMap<PathBuf, (u64, Instant)>,
}

#[cfg(target_os = "linux")]
enum CgroupKind {
    V2(CgroupLocation),
    V1 {
        cpu: Option<CgroupLocation>,
        cpuacct: Option<CgroupLocation>,
        memory: Option<CgroupLocation>,
    },
}

#[cfg(target_os = "linux")]
#[derive(Clone)]
struct CgroupLocation {
    root: PathBuf,
    mount_point: PathBuf,
    group_path: PathBuf,
}

#[cfg(target_os = "linux")]
struct CgroupMount {
    filesystem: String,
    root: PathBuf,
    mount_point: PathBuf,
    controllers: Vec<String>,
}

#[cfg(target_os = "linux")]
impl CgroupTelemetry {
    fn discover() -> io::Result<Option<Self>> {
        let Some(membership_contents) = read_optional(Path::new("/proc/self/cgroup"))? else {
            return Ok(None);
        };
        let Some(mount_contents) = read_optional(Path::new("/proc/self/mountinfo"))? else {
            return Ok(None);
        };
        let mounts = parse_mountinfo(&mount_contents);
        let memberships = parse_memberships(&membership_contents);

        if let Some(group_path) = memberships
            .iter()
            .find(|entry| entry.controllers.is_empty())
        {
            if let Some(location) = mounts
                .iter()
                .filter(|mount| mount.filesystem == "cgroup2")
                .find_map(|mount| location_for(mount, &group_path.path))
            {
                return Ok(Some(Self {
                    kind: CgroupKind::V2(location),
                    previous_cpu: HashMap::new(),
                }));
            }
        }

        let location = |controller: &str| {
            let group = memberships
                .iter()
                .find(|entry| entry.controllers.iter().any(|value| value == controller))?;
            mounts
                .iter()
                .filter(|mount| {
                    mount.filesystem == "cgroup"
                        && mount.controllers.iter().any(|value| value == controller)
                })
                .find_map(|mount| location_for(mount, &group.path))
        };
        let cpu = location("cpu");
        let cpuacct = location("cpuacct");
        let memory = location("memory");
        if cpu.is_none() && cpuacct.is_none() && memory.is_none() {
            return Ok(None);
        }

        Ok(Some(Self {
            kind: CgroupKind::V1 {
                cpu,
                cpuacct,
                memory,
            },
            previous_cpu: HashMap::new(),
        }))
    }

    fn prime_cpu_counters(&mut self) -> io::Result<()> {
        let now = Instant::now();
        for (path, usage, _) in self.cpu_counters()? {
            self.previous_cpu.insert(path, (usage, now));
        }
        Ok(())
    }

    fn sample_cpu_ratio(&mut self) -> io::Result<Option<f32>> {
        let now = Instant::now();
        let counters = self.cpu_counters()?;
        let mut ratios = Vec::new();
        for (path, usage, quota_cores) in counters {
            if let Some((previous_usage, previous_at)) =
                self.previous_cpu.insert(path, (usage, now))
            {
                if usage >= previous_usage {
                    if let Some(ratio) = cpu_usage_ratio(
                        usage - previous_usage,
                        now.duration_since(previous_at),
                        quota_cores,
                    ) {
                        ratios.push(ratio);
                    }
                }
            }
        }
        Ok(ratios.into_iter().reduce(f32::max))
    }

    fn sample_memory_ratio(&self) -> io::Result<Option<f32>> {
        let mut ratios = Vec::new();
        match &self.kind {
            CgroupKind::V2(location) => {
                for group in ancestor_groups(location) {
                    let Some(directory) = resolve_location(location, &group) else {
                        continue;
                    };
                    let limit = read_optional_memory_limit(&directory.join("memory.max"))?;
                    let used = read_optional_u64(&directory.join("memory.current"))?;
                    if let (Some(limit), Some(used)) = (limit, used) {
                        if is_finite_memory_limit(limit) {
                            ratios.push(byte_ratio(used, limit));
                        }
                    }
                }
            }
            CgroupKind::V1 { memory, .. } => {
                if let Some(location) = memory {
                    for group in ancestor_groups(location) {
                        let Some(directory) = resolve_location(location, &group) else {
                            continue;
                        };
                        let limit = read_optional_u64(&directory.join("memory.limit_in_bytes"))?;
                        let used = read_optional_u64(&directory.join("memory.usage_in_bytes"))?;
                        if let (Some(limit), Some(used)) = (limit, used) {
                            if is_finite_memory_limit(limit) {
                                ratios.push(byte_ratio(used, limit));
                            }
                        }
                    }
                }
            }
        }
        Ok(ratios.into_iter().reduce(f32::max))
    }

    fn cpu_counters(&self) -> io::Result<Vec<(PathBuf, u64, f64)>> {
        let mut counters = Vec::new();
        match &self.kind {
            CgroupKind::V2(location) => {
                for group in ancestor_groups(location) {
                    let Some(directory) = resolve_location(location, &group) else {
                        continue;
                    };
                    let quota = read_optional(&directory.join("cpu.max"))?
                        .map(|value| parse_v2_cpu_max(&value))
                        .transpose()?
                        .flatten();
                    if let Some(usage) = read_optional(&directory.join("cpu.stat"))?
                        .map(|value| parse_v2_cpu_usage(&value))
                        .transpose()?
                        .flatten()
                    {
                        counters.push((group, usage, quota.unwrap_or_else(available_cpu_count)));
                    }
                }
            }
            CgroupKind::V1 { cpu, cpuacct, .. } => {
                if let (Some(cpu), Some(cpuacct)) = (cpu, cpuacct) {
                    for group in ancestor_groups(cpu) {
                        let Some(cpu_directory) = resolve_location(cpu, &group) else {
                            continue;
                        };
                        let Some(cpuacct_directory) = resolve_location(cpuacct, &group) else {
                            continue;
                        };
                        let quota = read_optional_i64(&cpu_directory.join("cpu.cfs_quota_us"))?;
                        let period = read_optional_u64(&cpu_directory.join("cpu.cfs_period_us"))?;
                        if let (Some(quota), Some(period)) = (quota, period) {
                            let quota_cores = parse_v1_cpu_quota(quota, period)?;
                            if let Some(usage_ns) =
                                read_optional_u64(&cpuacct_directory.join("cpuacct.usage"))?
                            {
                                counters.push((group, usage_ns / 1_000, quota_cores));
                            }
                        }
                    }
                }
            }
        }
        Ok(counters)
    }
}

#[cfg(target_os = "linux")]
struct CgroupMembership {
    controllers: Vec<String>,
    path: PathBuf,
}

#[cfg(target_os = "linux")]
fn parse_memberships(contents: &str) -> Vec<CgroupMembership> {
    contents
        .lines()
        .filter_map(|line| {
            let mut fields = line.splitn(3, ':');
            let _hierarchy = fields.next()?;
            let controllers = fields
                .next()?
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            let path = fields.next()?;
            Some(CgroupMembership {
                controllers,
                path: PathBuf::from(path),
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn parse_mountinfo(contents: &str) -> Vec<CgroupMount> {
    contents
        .lines()
        .filter_map(|line| {
            let (left, right) = line.split_once(" - ")?;
            let left: Vec<_> = left.split_whitespace().collect();
            let right: Vec<_> = right.split_whitespace().collect();
            if left.len() < 5 || right.len() < 3 {
                return None;
            }
            let filesystem = right[0];
            if filesystem != "cgroup" && filesystem != "cgroup2" {
                return None;
            }
            let controllers = right[2]
                .split(',')
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
                .collect();
            Some(CgroupMount {
                filesystem: filesystem.to_owned(),
                root: PathBuf::from(decode_mount_path(left[3])),
                mount_point: PathBuf::from(decode_mount_path(left[4])),
                controllers,
            })
        })
        .collect()
}

#[cfg(target_os = "linux")]
fn decode_mount_path(value: &str) -> String {
    value
        .replace("\\040", " ")
        .replace("\\011", "\t")
        .replace("\\012", "\n")
        .replace("\\134", "\\")
}

#[cfg(target_os = "linux")]
fn location_for(mount: &CgroupMount, group_path: &Path) -> Option<CgroupLocation> {
    group_path.strip_prefix(&mount.root).ok()?;
    Some(CgroupLocation {
        root: mount.root.clone(),
        mount_point: mount.mount_point.clone(),
        group_path: group_path.to_path_buf(),
    })
}

#[cfg(target_os = "linux")]
fn ancestor_groups(location: &CgroupLocation) -> Vec<PathBuf> {
    let mut groups = Vec::new();
    let mut group = location.group_path.as_path();
    loop {
        if !group.starts_with(&location.root) {
            break;
        }
        groups.push(group.to_path_buf());
        if group == location.root {
            break;
        }
        let Some(parent) = group.parent() else {
            break;
        };
        group = parent;
    }
    groups
}

#[cfg(target_os = "linux")]
fn resolve_location(location: &CgroupLocation, group: &Path) -> Option<PathBuf> {
    let relative = group.strip_prefix(&location.root).ok()?;
    Some(location.mount_point.join(relative))
}

#[cfg(target_os = "linux")]
fn parse_v2_cpu_max(value: &str) -> io::Result<Option<f64>> {
    let mut fields = value.split_whitespace();
    let quota = fields.next().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidData, "missing CPU quota in cpu.max")
    })?;
    let period = fields
        .next()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "missing period in cpu.max"))?
        .parse::<u64>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if period == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CPU period in cpu.max must be greater than zero",
        ));
    }
    if quota == "max" {
        return Ok(None);
    }
    let quota = quota
        .parse::<u64>()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    if quota == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CPU quota in cpu.max must be greater than zero",
        ));
    }
    Ok(Some(quota as f64 / period as f64))
}

#[cfg(target_os = "linux")]
fn parse_v1_cpu_quota(quota: i64, period: u64) -> io::Result<f64> {
    if period == 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CPU period in cgroup v1 must be greater than zero",
        ));
    }
    if quota == -1 {
        return Ok(available_cpu_count());
    }
    if quota <= 0 {
        return Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "CPU quota in cgroup v1 must be positive or -1",
        ));
    }
    Ok(quota as f64 / period as f64)
}

#[cfg(target_os = "linux")]
fn parse_v2_cpu_usage(value: &str) -> io::Result<Option<u64>> {
    value
        .lines()
        .find_map(|line| {
            let (key, value) = line.split_once(' ')?;
            (key == "usage_usec").then_some(value.trim())
        })
        .map(|value| {
            value
                .parse::<u64>()
                .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
        })
        .transpose()
}

#[cfg(target_os = "linux")]
fn available_cpu_count() -> f64 {
    std::thread::available_parallelism()
        .map(|count| count.get() as f64)
        .unwrap_or(1.0)
}

#[cfg(target_os = "linux")]
fn byte_ratio(used: u64, total: u64) -> f32 {
    (used as f64 / total as f64).clamp(0.0, 1.0) as f32
}

#[cfg(target_os = "linux")]
fn is_finite_memory_limit(limit: u64) -> bool {
    limit > 0 && limit < (1_u64 << 60)
}

#[cfg(target_os = "linux")]
fn read_optional(path: &Path) -> io::Result<Option<String>> {
    match std::fs::read_to_string(path) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
        Err(error) => Err(error),
    }
}

#[cfg(target_os = "linux")]
fn read_optional_u64(path: &Path) -> io::Result<Option<u64>> {
    let Some(value) = read_optional(path)? else {
        return Ok(None);
    };
    value
        .trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "linux")]
fn read_optional_memory_limit(path: &Path) -> io::Result<Option<u64>> {
    let Some(value) = read_optional(path)? else {
        return Ok(None);
    };
    if value.trim() == "max" {
        return Ok(None);
    }
    value
        .trim()
        .parse::<u64>()
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(target_os = "linux")]
fn read_optional_i64(path: &Path) -> io::Result<Option<i64>> {
    let Some(value) = read_optional(path)? else {
        return Ok(None);
    };
    value
        .trim()
        .parse::<i64>()
        .map(Some)
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    use std::sync::atomic::{AtomicU64, Ordering};

    #[test]
    fn utilization_ratios_are_finite_and_bounded() {
        assert_eq!(percentage_to_ratio(150.0), Some(1.0));
        assert_eq!(percentage_to_ratio(-1.0), Some(0.0));
        assert_eq!(percentage_to_ratio(f32::NAN), None);
        assert_eq!(usage_ratio(50, 100), Some(0.5));
        assert_eq!(usage_ratio(1, 0), None);
    }

    #[test]
    fn cpu_usage_is_normalized_to_elapsed_time_and_quota() {
        assert_eq!(
            cpu_usage_ratio(250_000, Duration::from_millis(250), 2.0),
            Some(0.5)
        );
        assert_eq!(
            cpu_usage_ratio(1_000_000, Duration::from_millis(250), 1.0),
            Some(1.0)
        );
        assert_eq!(cpu_usage_ratio(0, Duration::ZERO, 1.0), None);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_cgroup_v2_quota_and_usage() {
        assert_eq!(parse_v2_cpu_max("200000 100000").unwrap(), Some(2.0));
        assert_eq!(parse_v2_cpu_max("max 100000").unwrap(), None);
        assert!(parse_v2_cpu_max("100000 0").is_err());
        assert!(parse_v2_cpu_max("0 100000").is_err());
        assert_eq!(
            parse_v2_cpu_usage("usage_usec 1234\nuser_usec 900\n").unwrap(),
            Some(1234)
        );
        assert!(parse_v2_cpu_max("invalid 100000").is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn parses_cgroup_v1_cpu_quota() {
        assert_eq!(parse_v1_cpu_quota(200_000, 100_000).unwrap(), 2.0);
        assert!(parse_v1_cpu_quota(-1, 100_000).unwrap() >= 1.0);
        assert!(parse_v1_cpu_quota(0, 100_000).is_err());
        assert!(parse_v1_cpu_quota(100_000, 0).is_err());
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn maps_cgroup_memberships_and_mounts() {
        let memberships = parse_memberships(
            "0::/system.slice/arc.service\n2:cpu,cpuacct:/system.slice/arc.service\n",
        );
        assert_eq!(memberships.len(), 2);
        assert!(memberships[0].controllers.is_empty());
        assert_eq!(
            memberships[0].path,
            PathBuf::from("/system.slice/arc.service")
        );

        let mounts = parse_mountinfo(
            "29 23 0:26 / /sys/fs/cgroup rw - cgroup2 cgroup rw\n\
             30 23 0:27 / /sys/fs/cgroup/cpu,cpuacct rw - cgroup cgroup rw,cpu,cpuacct\n",
        );
        assert_eq!(mounts.len(), 2);
        assert_eq!(mounts[0].filesystem, "cgroup2");
        assert!(mounts[1].controllers.contains(&"cpu".to_owned()));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn memory_limits_ignore_kernel_unlimited_sentinel() {
        assert!(is_finite_memory_limit(1024));
        assert!(!is_finite_memory_limit(1_u64 << 60));
        assert_eq!(byte_ratio(120, 100), 1.0);
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_cgroup_v2_metrics_from_limited_ancestors() {
        let directory = TestDirectory::new();
        directory.write("cpu.max", "200000 100000\n");
        directory.write("cpu.stat", "usage_usec 1000\n");
        directory.write("memory.max", "2000\n");
        directory.write("memory.current", "1000\n");
        directory.write("worker/cpu.max", "max 100000\n");
        directory.write("worker/cpu.stat", "usage_usec 2500\n");
        directory.write("worker/memory.max", "max\n");
        directory.write("worker/memory.current", "1500\n");

        let location = CgroupLocation {
            root: PathBuf::from("/"),
            mount_point: directory.path.clone(),
            group_path: PathBuf::from("/worker"),
        };
        let cgroups = CgroupTelemetry {
            kind: CgroupKind::V2(location),
            previous_cpu: HashMap::new(),
        };

        assert_eq!(cgroups.sample_memory_ratio().unwrap(), Some(0.5));
        let counters = cgroups.cpu_counters().unwrap();
        assert_eq!(counters.len(), 2);
        assert!(counters
            .iter()
            .any(|(_, usage, quota)| *usage == 1000 && *quota == 2.0));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn reads_cgroup_v1_metrics_from_limited_ancestors() {
        let directory = TestDirectory::new();
        directory.write("cpu/cpu.cfs_quota_us", "100000\n");
        directory.write("cpu/cpu.cfs_period_us", "100000\n");
        directory.write("cpuacct/cpuacct.usage", "1000000\n");
        directory.write("memory/memory.limit_in_bytes", "2000\n");
        directory.write("memory/memory.usage_in_bytes", "500\n");
        directory.write("cpu/worker/cpu.cfs_quota_us", "-1\n");
        directory.write("cpu/worker/cpu.cfs_period_us", "100000\n");
        directory.write("cpuacct/worker/cpuacct.usage", "1500000\n");
        directory.write(
            "memory/worker/memory.limit_in_bytes",
            "9223372036854771712\n",
        );
        directory.write("memory/worker/memory.usage_in_bytes", "1000\n");

        let location = |controller: &str| CgroupLocation {
            root: PathBuf::from("/"),
            mount_point: directory.path.join(controller),
            group_path: PathBuf::from("/worker"),
        };
        let cgroups = CgroupTelemetry {
            kind: CgroupKind::V1 {
                cpu: Some(location("cpu")),
                cpuacct: Some(location("cpuacct")),
                memory: Some(location("memory")),
            },
            previous_cpu: HashMap::new(),
        };

        assert_eq!(cgroups.sample_memory_ratio().unwrap(), Some(0.25));
        let counters = cgroups.cpu_counters().unwrap();
        assert_eq!(counters.len(), 2);
        assert!(counters
            .iter()
            .any(|(_, usage, quota)| *usage == 1000 && *quota == 1.0));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn samples_current_linux_host_or_container_resources() {
        let mut telemetry = ResourceTelemetry::new().unwrap();
        std::thread::sleep(CPU_SAMPLE_INTERVAL);
        let usage = telemetry.sample().unwrap();
        assert!(usage.cpu_utilization_ratio.is_some());
        assert!(usage.memory_utilization_ratio.is_some());
        assert!((0.0..=1.0).contains(&usage.cpu_utilization_ratio.unwrap()));
        assert!((0.0..=1.0).contains(&usage.memory_utilization_ratio.unwrap()));
    }

    #[cfg(target_os = "linux")]
    struct TestDirectory {
        path: PathBuf,
    }

    #[cfg(target_os = "linux")]
    impl TestDirectory {
        fn new() -> Self {
            static NEXT_ID: AtomicU64 = AtomicU64::new(0);
            let path = std::env::temp_dir().join(format!(
                "arc-cgroup-telemetry-{}-{}",
                std::process::id(),
                NEXT_ID.fetch_add(1, Ordering::Relaxed)
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn write(&self, relative: &str, contents: &str) {
            let path = self.path.join(relative);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, contents).unwrap();
        }
    }

    #[cfg(target_os = "linux")]
    impl Drop for TestDirectory {
        fn drop(&mut self) {
            std::fs::remove_dir_all(&self.path).unwrap();
        }
    }
}
