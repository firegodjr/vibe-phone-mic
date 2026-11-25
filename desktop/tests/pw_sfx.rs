//! Click sounds really reach the default output of a PRIVATE PipeWire instance (tools/pw-private.sh), and the
//! playback stream closes again when idle. Skips with a message when the private instance can't be started.

mod common;

use common::pw::private_pw;
use std::process::Stdio;
use std::time::Duration;
use vibe_phone_mic::sfx::{Sfx, Sound};

#[test]
fn clicks_play_on_default_sink_and_stream_closes_when_idle() {
    let Some(pw) = private_pw() else { return };
    pw.apply_to_process();
    let mut rec = pw
        .cmd("parec")
        .args(["--latency-msec=20", "--device", "@DEFAULT_MONITOR@", "--format=s16le", "--rate=48000", "--channels=1"])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn parec");
    let mut out = rec.stdout.take().unwrap();
    let reader = std::thread::spawn(move || {
        let mut v = Vec::new();
        let _ = std::io::Read::read_to_end(&mut out, &mut v);
        v
    });
    std::thread::sleep(Duration::from_millis(300));

    let sfx = Sfx::pulse();
    for _ in 0..3 {
        for s in Sound::ALL {
            sfx.play(s, true);
            std::thread::sleep(Duration::from_millis(80));
            sfx.play(s, false);
            std::thread::sleep(Duration::from_millis(150));
        }
    }
    std::thread::sleep(Duration::from_millis(300));
    assert!(pw.pactl(&["list", "short", "sink-inputs"]).lines().count() >= 1, "no playback stream while clicking");

    let _ = rec.kill();
    let _ = rec.wait();
    let bytes = reader.join().unwrap();
    let samples: Vec<i16> = bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect();
    let peak = samples.iter().map(|s| s.unsigned_abs()).max().unwrap_or(0);
    // Quietest clip: carson at 0.25 gain ≈ 0.11 FS peak; kvm at 0.9 ≈ 0.07–0.15 FS. Silence is 0.
    assert!(peak > 1500, "clicks not audible on the default sink monitor (peak {peak}, {} samples)", samples.len());

    // Idle: the stream is closed again after IDLE_CLOSE (4 s).
    pw.wait_for("sfx stream closed", Duration::from_secs(8), || {
        pw.pactl(&["list", "short", "sink-inputs"]).trim().is_empty()
    });
}
