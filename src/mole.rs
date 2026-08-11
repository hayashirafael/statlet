use std::cmp::Ordering;
use std::collections::HashSet;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering as AtomicOrdering};
use std::thread;
use std::time::{Duration, Instant};

pub const MINIMUM_MOLE_VERSION: MoleVersion = MoleVersion::new(1, 39, 1);
const DEFAULT_VERSION_TIMEOUT: Duration = Duration::from_secs(2);
const TERMINAL_LAUNCH_TIMEOUT: Duration = Duration::from_secs(5);
const VERSION_POLL_INTERVAL: Duration = Duration::from_millis(10);
const MAX_VERSION_OUTPUT_BYTES: u64 = 4 * 1024;
static NEXT_TEMP_OUTPUT_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum MoleStatus {
    Unknown,
    Compatible(MoleVersion),
    Missing,
    Unavailable,
    Incompatible(MoleVersion),
}

impl MoleStatus {
    pub const fn is_compatible(self) -> bool {
        matches!(self, Self::Compatible(_))
    }

    pub const fn is_error(self) -> bool {
        matches!(
            self,
            Self::Missing | Self::Unavailable | Self::Incompatible(_)
        )
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct MoleVersion {
    pub major: u64,
    pub minor: u64,
    pub patch: u64,
}

impl MoleVersion {
    pub const fn new(major: u64, minor: u64, patch: u64) -> Self {
        Self {
            major,
            minor,
            patch,
        }
    }

    pub fn is_supported(self) -> bool {
        self.major == 1 && self.cmp(&MINIMUM_MOLE_VERSION) != Ordering::Less
    }
}

impl Ord for MoleVersion {
    fn cmp(&self, other: &Self) -> Ordering {
        (self.major, self.minor, self.patch).cmp(&(other.major, other.minor, other.patch))
    }
}

impl PartialOrd for MoleVersion {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoleInstallation {
    executable: PathBuf,
    version: MoleVersion,
}

impl MoleInstallation {
    pub fn executable(&self) -> &Path {
        &self.executable
    }

    pub const fn version(&self) -> MoleVersion {
        self.version
    }

    pub fn terminal_launch_plan(&self) -> TerminalLaunchPlan {
        TerminalLaunchPlan {
            program: PathBuf::from("/usr/bin/osascript"),
            arguments: vec![
                OsString::from("-e"),
                OsString::from(
                    "on run argv\nset molePath to item 1 of argv\ntell application \"Terminal\"\nactivate\ndo script (quoted form of molePath) & \" clean\"\nend tell\nend run",
                ),
                OsString::from("--"),
                self.executable.as_os_str().to_owned(),
            ],
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct MoleDetection {
    pub status: MoleStatus,
    pub installation: Option<MoleInstallation>,
}

#[derive(Clone)]
pub struct MoleDetector {
    candidates: Vec<PathBuf>,
    version_timeout: Duration,
}

impl MoleDetector {
    pub fn system() -> Self {
        let mut candidates = Vec::new();
        if let Some(path) = std::env::var_os("PATH") {
            candidates.extend(std::env::split_paths(&path).map(|directory| directory.join("mo")));
        }
        candidates.push(PathBuf::from("/opt/homebrew/bin/mo"));
        candidates.push(PathBuf::from("/usr/local/bin/mo"));
        if let Some(home) = std::env::var_os("HOME") {
            candidates.push(PathBuf::from(home).join(".local/bin/mo"));
        }
        Self::from_candidates(candidates)
    }

    pub fn from_candidates(candidates: impl IntoIterator<Item = PathBuf>) -> Self {
        Self::from_candidates_with_timeout(candidates, DEFAULT_VERSION_TIMEOUT)
    }

    pub fn from_candidates_with_timeout(
        candidates: impl IntoIterator<Item = PathBuf>,
        version_timeout: Duration,
    ) -> Self {
        let mut seen = HashSet::new();
        Self {
            candidates: candidates
                .into_iter()
                .filter(|candidate| seen.insert(candidate.clone()))
                .collect(),
            version_timeout,
        }
    }

    pub fn detect(&self) -> MoleDetection {
        let mut found_candidate = false;
        let mut first_incompatible = None;
        for executable in self
            .candidates
            .iter()
            .filter(|candidate| candidate.is_file())
        {
            found_candidate = true;
            let Some(stdout) = run_version_command(executable, self.version_timeout) else {
                continue;
            };
            let Some(version) = parse_version(&String::from_utf8_lossy(&stdout)) else {
                continue;
            };
            if !version.is_supported() {
                first_incompatible.get_or_insert(version);
                continue;
            }
            return MoleDetection {
                status: MoleStatus::Compatible(version),
                installation: Some(MoleInstallation {
                    executable: executable.clone(),
                    version,
                }),
            };
        }

        if let Some(version) = first_incompatible {
            MoleDetection {
                status: MoleStatus::Incompatible(version),
                installation: None,
            }
        } else if found_candidate {
            unavailable_detection()
        } else {
            MoleDetection {
                status: MoleStatus::Missing,
                installation: None,
            }
        }
    }
}

fn run_version_command(executable: &Path, timeout: Duration) -> Option<Vec<u8>> {
    let mut output = TemporaryOutput::new().ok()?;
    let mut command = Command::new(executable);
    command
        .arg("--version")
        .env("NO_COLOR", "1")
        .stdout(Stdio::from(output.file.try_clone().ok()?))
        .stderr(Stdio::null())
        .process_group(0);
    // SAFETY: this runs only in the child before exec, calls the async-signal-safe `setrlimit`,
    // and caps version output for that child and its descendants without changing the app.
    unsafe {
        command.pre_exec(|| {
            let limit = libc::rlimit {
                rlim_cur: MAX_VERSION_OUTPUT_BYTES as libc::rlim_t,
                rlim_max: MAX_VERSION_OUTPUT_BYTES as libc::rlim_t,
            };
            if libc::setrlimit(libc::RLIMIT_FSIZE, &limit) == 0 {
                Ok(())
            } else {
                Err(std::io::Error::last_os_error())
            }
        });
    }
    let mut child = command.spawn().ok()?;
    let status = wait_for_child(&mut child, timeout).ok()?;
    // `mo --version` is a bounded probe. A successful leader must not leave background
    // descendants behind; terminate the private group before consuming its captured output.
    kill_process_group(&mut child);
    if !status.success() {
        return None;
    }
    output.file.seek(SeekFrom::Start(0)).ok()?;
    let mut stdout = Vec::new();
    (&mut output.file)
        .take(MAX_VERSION_OUTPUT_BYTES + 1)
        .read_to_end(&mut stdout)
        .ok()?;
    (stdout.len() as u64 <= MAX_VERSION_OUTPUT_BYTES).then_some(stdout)
}

struct TemporaryOutput {
    path: PathBuf,
    file: File,
}

impl TemporaryOutput {
    fn new() -> std::io::Result<Self> {
        for _ in 0..16 {
            let id = NEXT_TEMP_OUTPUT_ID.fetch_add(1, AtomicOrdering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "statlet-mo-version-{}-{id}.tmp",
                std::process::id()
            ));
            match OpenOptions::new()
                .read(true)
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&path)
            {
                Ok(file) => return Ok(Self { path, file }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a temporary Mole version output",
        ))
    }
}

impl Drop for TemporaryOutput {
    fn drop(&mut self) {
        let _ = fs::remove_file(&self.path);
    }
}

fn unavailable_detection() -> MoleDetection {
    MoleDetection {
        status: MoleStatus::Unavailable,
        installation: None,
    }
}

fn parse_version(output: &str) -> Option<MoleVersion> {
    output.split_whitespace().find_map(|token| {
        let token = token.trim_start_matches(['v', 'V']);
        let mut parts = token.split('.');
        let version = MoleVersion::new(
            parts.next()?.parse().ok()?,
            parts.next()?.parse().ok()?,
            parts
                .next()?
                .trim_end_matches(|character: char| !character.is_ascii_digit())
                .parse()
                .ok()?,
        );
        parts.next().is_none().then_some(version)
    })
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct TerminalLaunchPlan {
    pub program: PathBuf,
    pub arguments: Vec<OsString>,
}

impl TerminalLaunchPlan {
    pub fn launch(&self) -> std::io::Result<ExitStatus> {
        self.launch_with_timeout(TERMINAL_LAUNCH_TIMEOUT)
    }

    pub fn launch_with_timeout(&self, timeout: Duration) -> std::io::Result<ExitStatus> {
        let mut child = Command::new(&self.program)
            .args(&self.arguments)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .process_group(0)
            .spawn()?;
        wait_for_child(&mut child, timeout)
    }
}

fn wait_for_child(child: &mut Child, timeout: Duration) -> std::io::Result<ExitStatus> {
    let deadline = Instant::now()
        .checked_add(timeout)
        .ok_or_else(|| std::io::Error::other("subprocess timeout overflow"))?;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return Ok(status),
            Ok(None) if Instant::now() < deadline => thread::sleep(VERSION_POLL_INTERVAL),
            Ok(None) => {
                kill_process_group(child);
                let _ = child.wait();
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "subprocess did not exit before its deadline",
                ));
            }
            Err(error) => {
                kill_process_group(child);
                let _ = child.wait();
                return Err(error);
            }
        }
    }
}

fn kill_process_group(child: &mut Child) {
    if let Ok(group) = i32::try_from(child.id()) {
        // SAFETY: all children passed here were spawned in a new process group whose id is the
        // child's pid. The negative pid targets only that group, including descendants.
        unsafe {
            libc::kill(-group, libc::SIGKILL);
        }
    } else {
        let _ = child.kill();
    }
}
