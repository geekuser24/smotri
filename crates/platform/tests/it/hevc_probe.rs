//! `register()` answers the HEVC availability probe in the background, so the first draw of the
//! format list does not create a hardware session on the UI thread. One test in this file on
//! purpose: anything else that asked `hevc_available()` first would make the wait below meaningless.
#![cfg(target_os = "macos")]

use std::time::{Duration, Instant};

use filmcraft_platform::hardware_encode::{hevc_available, hevc_probed, warm_hevc_probe};

#[test]
fn register_answers_the_hevc_probe_in_the_background() {
    // Other modules in this integration-test binary initialize the same global probe.
    // A fresh process proves the cold-start contract without racing their encoder tests.
    if std::env::var_os("SMOTRI_HEVC_PROBE_CHILD").is_none() {
        let output = std::process::Command::new(std::env::current_exe().expect("test executable"))
            .args(["--exact", "hevc_probe::register_answers_the_hevc_probe_in_the_background", "--nocapture"])
            .env("SMOTRI_HEVC_PROBE_CHILD", "1")
            .output()
            .expect("isolated probe test");
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        return;
    }
    assert!(!hevc_probed(), "nothing has asked yet");

    filmcraft_platform::register();

    // nobody calls `hevc_available()` here: only the thread `register` started can answer
    let t0 = Instant::now();
    while !hevc_probed() {
        assert!(t0.elapsed() < Duration::from_secs(60), "register() did not start the probe");
        std::thread::sleep(Duration::from_millis(5));
    }

    // asking again, or registering again, changes nothing
    let answer = hevc_available();
    warm_hevc_probe();
    filmcraft_platform::register();
    assert_eq!(hevc_available(), answer);
    assert!(hevc_probed());
}
