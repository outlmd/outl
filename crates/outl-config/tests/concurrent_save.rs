//! Two clients saving at once must never publish a truncated config.
//!
//! `~/.config/outl/config.toml` is one file with several writers by
//! design — the TUI and the desktop app read and write the same path, and
//! a user runs both. `save_to` publishes by rename, which is atomic, so
//! the published slot can only ever hold a whole body **as long as every
//! writer composes in a scratch file of its own**.
//!
//! With one shared scratch name they share an *inode*: writer B's
//! `File::create` truncates the body A already fsynced, A's `rename`
//! publishes those zero bytes, and B then writes through a descriptor
//! that now points at the published `config.toml` while its own rename
//! fails `ENOENT`. The user sees "could not write", and what is left on
//! disk is a zero-byte config.
//!
//! That last part is what ties this to issue #284: every field carries
//! `#[serde(default)]`, so a zero-byte file is `ConfigSource::Parsed`,
//! not `Unreadable`. The write guard added for #284 sees nothing wrong
//! with it and the next save writes defaults over the lot — the same loss
//! that guard exists to stop, reached by a door it does not watch.
//!
//! **Do not relax these into "the final file parses".** A late rename can
//! paper over an earlier zero-byte publish, so the reader thread and the
//! per-save `Ok` are the assertions that actually catch the race.

use std::fs;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::thread;

use outl_config::Config;
use tempfile::TempDir;

/// Eight writers, forty saves each.
///
/// Not arbitrary: the collision window is the span between a writer's
/// `File::create` and its `rename`, and that span contains an `fsync`, so
/// it is milliseconds wide and any two concurrent writers overlap inside
/// it almost every time. Eight is enough to keep several writers in that
/// window at once on a single-core CI runner; forty rounds keeps the whole
/// test in the low seconds with the two `fsync`s each save pays.
const WRITERS: usize = 8;
const SAVES_PER_WRITER: usize = 40;

fn config_with(preset: &str) -> Config {
    let mut cfg = Config::default();
    cfg.theme.preset = preset.to_string();
    cfg
}

#[test]
fn concurrent_saves_never_publish_a_truncated_config() {
    let tmp = TempDir::new().expect("tempdir");
    let path = tmp.path().join("config.toml");
    outl_config::save_to(&path, &config_with("seed")).expect("seed a readable config");

    let stop = Arc::new(AtomicBool::new(false));
    let empty_reads = Arc::new(AtomicUsize::new(0));
    let unparseable_reads = Arc::new(AtomicUsize::new(0));

    // Two readers standing in for every process that reads this file
    // while another writes it — `outl doctor`, a second client booting,
    // `save_to`'s own guard.
    let readers: Vec<_> = (0..2)
        .map(|_| {
            let path = path.clone();
            let stop = Arc::clone(&stop);
            let empty_reads = Arc::clone(&empty_reads);
            let unparseable_reads = Arc::clone(&unparseable_reads);
            thread::spawn(move || {
                while !stop.load(Ordering::Relaxed) {
                    match fs::read_to_string(&path) {
                        Ok(raw) if raw.is_empty() => {
                            empty_reads.fetch_add(1, Ordering::Relaxed);
                        }
                        Ok(raw) => {
                            if toml::from_str::<toml::Value>(&raw).is_err() {
                                unparseable_reads.fetch_add(1, Ordering::Relaxed);
                            }
                        }
                        // A reader racing the publish is allowed to miss
                        // the file entirely on some platforms; that is not
                        // the defect under test.
                        Err(_) => {}
                    }
                }
            })
        })
        .collect();

    let writers: Vec<_> = (0..WRITERS)
        .map(|w| {
            let path = path.clone();
            thread::spawn(move || {
                let cfg = config_with(&format!("writer-{w}"));
                let mut failures = Vec::new();
                for _ in 0..SAVES_PER_WRITER {
                    if let Err(e) = outl_config::save_to(&path, &cfg) {
                        failures.push(e.to_string());
                    }
                }
                failures
            })
        })
        .collect();

    let failures: Vec<String> = writers
        .into_iter()
        .flat_map(|h| h.join().expect("writer thread"))
        .collect();
    stop.store(true, Ordering::Relaxed);
    for r in readers {
        r.join().expect("reader thread");
    }

    // All three verdicts at once rather than three asserts in a row: the
    // three are symptoms of one race, and seeing only the first one makes
    // a partial fix look like a whole one.
    let mut problems = Vec::new();
    let empty = empty_reads.load(Ordering::Relaxed);
    if empty > 0 {
        problems.push(format!(
            "{empty} reads saw a zero-byte config.toml. Every field carries \
             a serde default, so that reads back as a valid config of all \
             defaults and the #284 guard never fires"
        ));
    }
    let torn = unparseable_reads.load(Ordering::Relaxed);
    if torn > 0 {
        problems.push(format!(
            "{torn} reads saw a half-written config.toml — the publish was \
             not atomic"
        ));
    }
    if !failures.is_empty() {
        // Deduplicated: the race produces the same `ENOENT` hundreds of
        // times, and a panic message that long buries the other verdicts.
        let mut distinct: Vec<&String> = failures.iter().collect();
        distinct.sort();
        distinct.dedup();
        problems.push(format!(
            "{} of {} saves failed; concurrent writers are the normal case \
             for this file, not an error: {:?}",
            failures.len(),
            WRITERS * SAVES_PER_WRITER,
            distinct
        ));
    }
    assert!(problems.is_empty(), "{}", problems.join("\n"));

    let raw = fs::read_to_string(&path).expect("read the published config");
    assert!(!raw.is_empty(), "the published config must not be empty");
    let back = outl_config::load_result_from(&path);
    assert_eq!(
        back.source,
        outl_config::ConfigSource::Parsed,
        "the published config must parse"
    );
    assert!(
        (0..WRITERS).any(|w| back.config.theme.preset == format!("writer-{w}")),
        "the published config must be one writer's body, not a hybrid and \
         not defaults: {:?}",
        back.config.theme.preset
    );
}
