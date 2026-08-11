use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::time::{Duration, Instant};

use statlet::mole::{MoleDetector, MoleStatus, MoleVersion, TerminalLaunchPlan};
use tempfile::tempdir;

#[test]
fn detector_invokes_only_the_public_version_command() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo");
    let arguments_log = directory.path().join("arguments.log");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\nprintf '%s\\n' \"$@\" > '{}'\nprintf 'Mole version 1.49.2\\n'\n",
            arguments_log.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let detection = MoleDetector::from_candidates([executable.clone()]).detect();

    assert_eq!(
        detection.status,
        MoleStatus::Compatible(MoleVersion::new(1, 49, 2))
    );
    assert_eq!(
        detection.installation.unwrap().executable(),
        executable.as_path()
    );
    assert_eq!(fs::read_to_string(arguments_log).unwrap(), "--version\n");
}

#[test]
fn future_major_version_is_blocked_until_its_contract_is_reviewed() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo");
    fs::write(&executable, "#!/bin/sh\nprintf 'Mole version 2.0.0\\n'\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();

    let detection = MoleDetector::from_candidates([executable]).detect();

    assert_eq!(
        detection.status,
        MoleStatus::Incompatible(MoleVersion::new(2, 0, 0))
    );
    assert!(detection.installation.is_none());
}

#[test]
fn detector_continues_until_it_finds_a_compatible_installation() {
    let directory = tempdir().unwrap();
    let incompatible = directory.path().join("old-mo");
    let compatible = directory.path().join("current-mo");
    for (executable, version) in [(&incompatible, "1.20.0"), (&compatible, "1.49.2")] {
        fs::write(
            executable,
            format!("#!/bin/sh\nprintf 'Mole version {version}\\n'\n"),
        )
        .unwrap();
        let mut permissions = fs::metadata(executable).unwrap().permissions();
        permissions.set_mode(0o755);
        fs::set_permissions(executable, permissions).unwrap();
    }

    let detection = MoleDetector::from_candidates([incompatible, compatible.clone()]).detect();

    assert_eq!(
        detection.status,
        MoleStatus::Compatible(MoleVersion::new(1, 49, 2))
    );
    assert_eq!(
        detection.installation.unwrap().executable(),
        compatible.as_path()
    );
}

#[test]
fn terminal_plan_opens_only_the_official_interactive_mole_command() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo");
    fs::write(&executable, "#!/bin/sh\nprintf 'Mole version 1.49.2\\n'\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let detector = MoleDetector::from_candidates([executable]);
    let installation = detector
        .detect()
        .installation
        .expect("local Mole installation");

    let plan = installation.terminal_launch_plan();

    assert_eq!(plan.program.to_string_lossy(), "/usr/bin/osascript");
    assert_eq!(plan.arguments[0], "-e");
    let script = plan.arguments[1].to_string_lossy();
    assert!(script.contains("tell application \"Terminal\""));
    assert!(script.contains("quoted form of molePath"));
    assert!(script.contains("\" clean\""));
    for forbidden in ["--dry-run", "--json", "sudo", "--execute", "--yes"] {
        assert!(!script.contains(forbidden));
    }
}

#[test]
fn detector_times_out_a_stalled_version_command() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo");
    fs::write(&executable, "#!/bin/sh\nexec sleep 5\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let detector =
        MoleDetector::from_candidates_with_timeout([executable], Duration::from_millis(100));
    let started = Instant::now();

    let detection = detector.detect();

    assert_eq!(detection.status, MoleStatus::Unavailable);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn detector_does_not_wait_for_a_descendant_holding_stdout_open() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo");
    let descendant_marker = directory.path().join("descendant-survived");
    fs::write(
        &executable,
        format!(
            "#!/bin/sh\n(sleep 0.2; touch '{}') &\nprintf 'Mole version 1.49.2\\n'\n",
            descendant_marker.display()
        ),
    )
    .unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let detector =
        MoleDetector::from_candidates_with_timeout([executable], Duration::from_millis(500));
    let started = Instant::now();

    let detection = detector.detect();

    assert!(matches!(
        detection.status,
        MoleStatus::Compatible(MoleVersion {
            major: 1,
            minor: 49,
            patch: 2
        }) | MoleStatus::Unavailable
    ));
    assert!(started.elapsed() < Duration::from_secs(1));
    std::thread::sleep(Duration::from_millis(350));
    assert!(!descendant_marker.exists());
}

#[test]
fn terminal_launcher_times_out_instead_of_hanging_the_app() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("stalled-launcher");
    fs::write(&executable, "#!/bin/sh\nexec sleep 5\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let plan = TerminalLaunchPlan {
        program: executable,
        arguments: Vec::new(),
    };
    let started = Instant::now();

    let error = plan
        .launch_with_timeout(Duration::from_millis(100))
        .unwrap_err();

    assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    assert!(started.elapsed() < Duration::from_secs(1));
}

#[test]
fn terminal_plan_passes_an_adversarial_executable_path_as_data() {
    let directory = tempdir().unwrap();
    let executable = directory.path().join("mo\"\ndo shell script \"id\"");
    fs::write(&executable, "#!/bin/sh\nprintf 'Mole version 1.49.2\\n'\n").unwrap();
    let mut permissions = fs::metadata(&executable).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(&executable, permissions).unwrap();
    let installation = MoleDetector::from_candidates([executable.clone()])
        .detect()
        .installation
        .unwrap();

    let plan = installation.terminal_launch_plan();

    assert!(!plan.arguments[1]
        .to_string_lossy()
        .contains(executable.to_string_lossy().as_ref()));
    assert_eq!(plan.arguments[2], "--");
    assert_eq!(plan.arguments[3], executable.as_os_str());
}
