//! What a crash report may contain, decided field by field.
//!
//! This runs as `before_send`, which `sentry-core` invokes **last** — after
//! every integration's `process_event` and after `release` / `environment` /
//! `server_name` have been filled in from the client options. So it is the
//! final word on the payload, and the only thing that has to be right.
//!
//! The shape is an allowlist, not a denylist. A field is cleared unless RFC
//! 0377 §3 authorizes it, and [`ALLOWED_EVENT_KEYS`] names the survivors.
//! Clearing a field outl never writes (`tags`, `extra`, `modules`) is not
//! redundant: it is what keeps the promise if a later change enables the
//! `contexts` integration or starts tagging events.
//!
//! `contexts` is the one field this scrubber *writes*. It clears whatever
//! arrived and then puts back exactly two values, so the OS and CPU
//! architecture RFC §3 promises are present by construction rather than by
//! permission.

use std::borrow::Cow;

use sentry::protocol::{DebugImage, DebugMeta, DeviceContext, Event, OsContext, Stacktrace};

use crate::redact::redact;

/// The top-level keys a scrubbed event may serialize.
///
/// Everything here is either protocol plumbing (`event_id`, `level`,
/// `platform`, `timestamp`, `sdk`), a constant we chose (`release`,
/// `environment`, `contexts`), or scrubbed author-written text (`culprit`,
/// `transaction`, `message`, `logentry`, `exception`, `debug_meta`).
///
/// `contexts` is in the second group, not the first: it survives because this
/// scrubber rebuilds it from compile-time constants, not because an incoming
/// value was allowed through.
///
/// `an_event_carries_only_allowlisted_fields` asserts the serialized key set
/// against this list, so a field added to the protocol — or stopped being
/// cleared — fails a test instead of reaching a server.
pub const ALLOWED_EVENT_KEYS: &[&str] = &[
    "contexts",
    "culprit",
    "debug_meta",
    "environment",
    "event_id",
    "exception",
    "level",
    "logentry",
    "message",
    "platform",
    "release",
    "sdk",
    "timestamp",
    "transaction",
];

/// Strips everything a crash report has no business carrying, and redacts what
/// is left.
///
/// Never drops the event (`None` would lose the crash, which is the one thing
/// we are here to keep); it returns the same event with the payload reduced.
pub fn scrub_event(mut event: Event<'static>) -> Option<Event<'static>> {
    // Identity. `server_name` is already `None` in our options and the
    // `contexts` integration that would fill it from the hostname is not
    // enabled — this is the third line of defence, so that enabling that
    // feature later cannot leak a machine name by omission.
    event.server_name = None;
    event.user = None;
    event.request = None;

    // Free-form carriers outl never writes.
    event.extra.clear();
    event.tags.clear();
    event.contexts.clear();
    event.modules.clear();

    // `contexts` is rebuilt here, never passed through. RFC §3 promises the OS
    // and CPU architecture, and `std::env::consts` is where they come from:
    // compile-time constants, so reading them touches nothing on the running
    // machine — no hostname, no kernel version, no device model, none of what
    // the `contexts` integration would have collected alongside them.
    //
    // Writing them *after* the `clear()` above is what keeps this an allowlist
    // by construction. Enable that integration tomorrow and everything it
    // gathered still dies in the `clear()`, while these two fields are still
    // only what this function put here. An allowlist of keys *inside*
    // `contexts` would have the opposite property: it would trust what arrived.
    event.contexts.insert(
        "os".to_string(),
        OsContext {
            name: Some(std::env::consts::OS.to_string()),
            ..Default::default()
        }
        .into(),
    );
    event.contexts.insert(
        "device".to_string(),
        DeviceContext {
            arch: Some(std::env::consts::ARCH.to_string()),
            ..Default::default()
        }
        .into(),
    );
    event.breadcrumbs.values.clear();
    event.threads.values.clear();
    event.logger = None;
    event.dist = None;
    event.stacktrace = None;
    event.template = None;
    // Back to the protocol default rather than empty: an empty fingerprint
    // serializes as `[]` and tells the server to group every crash together.
    event.fingerprint = Event::default().fingerprint;

    // Author-written text. Ours, and therefore exactly where a page title
    // reaches a report: `expect(&format!("page {slug}"))` is our code.
    event.culprit = event.culprit.take().map(|culprit| redact(&culprit));
    event.transaction = event.transaction.take().map(|name| redact(&name));
    event.message = event.message.take().map(|message| redact(&message));

    if let Some(entry) = event.logentry.as_mut() {
        entry.message = redact(&entry.message);
        entry.params.clear();
    }

    for exception in event.exception.values.iter_mut() {
        // `ty` and `module` are Rust type paths, not data.
        exception.value = exception.value.take().map(|value| redact(&value));
        scrub_stacktrace(exception.stacktrace.as_mut());
        scrub_stacktrace(exception.raw_stacktrace.as_mut());
        if let Some(mechanism) = exception.mechanism.as_mut() {
            mechanism.description = mechanism.description.take().map(|text| redact(&text));
            mechanism.data.clear();
        }
    }

    scrub_debug_meta(&mut event.debug_meta);

    Some(event)
}

/// Keeps a stack frame readable while removing the parts that describe the
/// user's disk instead of our source tree.
///
/// `filename` survives [`redact`] untouched, and it is worth being precise
/// about what that is: `sentry-backtrace` fills it with the **basename**
/// (`mod.rs`), not a repo-relative path, and puts the absolute build path in
/// `abs_path`, which does not survive. So a frame leaves here as `mod.rs:42`.
///
/// This file used to say "usually the compile-time, repo-relative path". It is
/// not, and three user-facing texts repeated the same thing until a review
/// read `sentry-backtrace`'s `process.rs`. The readable location in Sentry is
/// reconstructed server-side from the debug files the release pipeline
/// uploads, which is also why symbolication does not depend on either field:
/// it runs off `instruction_addr` and the debug images.
fn scrub_stacktrace(stacktrace: Option<&mut Stacktrace>) {
    let Some(stacktrace) = stacktrace else {
        return;
    };
    for frame in stacktrace.frames.iter_mut() {
        frame.abs_path = frame.abs_path.take().map(|path| redact(&path));
        frame.filename = frame.filename.take().map(|path| redact(&path));
        // Source text and locals. Nothing populates these today; a frame
        // carrying the user's own block text is the worst case this crate has.
        frame.pre_context.clear();
        frame.post_context.clear();
        frame.context_line = None;
        frame.vars.clear();
    }
}

/// Redacts the image paths the `debug-images` integration collects.
///
/// Those `name` fields are absolute paths to the loaded binary and its
/// libraries, so on a `cargo install` or Homebrew machine they read
/// `/Users/<username>/.cargo/bin/outl` — the username, from a feature we
/// deliberately enable. The debug *id* is what Sentry matches against the
/// symbols uploaded by the release pipeline, and that is kept, so redacting the
/// path costs nothing.
///
/// The `match` is exhaustive on purpose: a new `DebugImage` variant should fail
/// to compile until somebody decides whether it carries a path.
fn scrub_debug_meta(meta: &mut Cow<'static, DebugMeta>) {
    // `sdk_info` goes, and it goes *before* the early return below, which is
    // the whole reason this is not one line further down.
    //
    // It used to be kept, with a comment calling it "narrower than the
    // `os.name` the scrubber writes". That was backwards. `SystemSdkInfo` is
    // a name plus major, minor and patchlevel — `macosx 15.0.0` — and RFC
    // §3 promises the OS "as a bare name, not its version, build or kernel".
    // Nothing in `sentry-debug-images` 0.49 fills it, so this clears a field
    // that arrives empty today; it is here for the same reason `server_name`
    // is, namely that the promise should survive the integration that starts
    // filling it.
    if meta.sdk_info.is_some() {
        meta.to_mut().sdk_info = None;
    }
    if meta.images.is_empty() {
        return;
    }
    for image in meta.to_mut().images.iter_mut() {
        match image {
            DebugImage::Apple(image) => image.name = redact(&image.name),
            DebugImage::Symbolic(image) => {
                image.name = redact(&image.name);
                image.debug_file = image.debug_file.take().map(|path| redact(&path));
            }
            DebugImage::Wasm(image) => {
                image.name = redact(&image.name);
                image.code_file = redact(&image.code_file);
                image.debug_file = image.debug_file.take().map(|path| redact(&path));
            }
            // A proguard reference is a bare UUID.
            DebugImage::Proguard(_) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;
    use std::time::SystemTime;

    use sentry::protocol::{
        Addr, Breadcrumb, ClientSdkInfo, Context, Exception, Frame, Level, LogEntry, Mechanism,
        Request, SymbolicDebugImage, SystemSdkInfo, TemplateInfo, Thread, User,
    };
    use sentry::types::{DebugId, Uuid};
    use serde_json::{json, Value};

    use super::*;

    const LEAK: &str = "/Users/avelino/outl-p2p/pages/terapia.md";

    /// Every `Event` field populated with something that must not be sent,
    /// then scrubbed, then serialized — so the assertion is over the bytes
    /// that would reach the wire, not over the struct.
    ///
    /// This is the test that makes "a field was added to the protocol" a
    /// compile-or-fail event rather than a leak nobody notices.
    #[test]
    fn an_event_carries_only_allowlisted_fields() {
        let scrubbed = scrub_event(fully_populated_event()).expect("the event is never dropped");
        let json = serde_json::to_value(&scrubbed).expect("serialize");

        let keys: BTreeSet<&str> = json
            .as_object()
            .expect("an event serializes as an object")
            .keys()
            .map(String::as_str)
            .collect();
        let allowed: BTreeSet<&str> = ALLOWED_EVENT_KEYS.iter().copied().collect();

        assert_eq!(
            keys, allowed,
            "the serialized key set drifted from ALLOWED_EVENT_KEYS; \
             a new key means a new thing leaving the user's machine"
        );
    }

    /// The key set alone would pass if a surviving field still held the leak,
    /// so the payload is checked for the string too, recursively.
    #[test]
    fn no_populated_field_leaves_a_path_behind() {
        let scrubbed = scrub_event(fully_populated_event()).expect("the event is never dropped");
        let json = serde_json::to_string(&scrubbed).expect("serialize");

        assert!(!json.contains("terapia"), "page title survived: {json}");
        assert!(!json.contains("avelino"), "username survived: {json}");
        assert!(
            !json.contains("outl-p2p"),
            "workspace name survived: {json}"
        );
    }

    /// The other direction: the crash itself has to arrive. A scrubber that
    /// returned an empty event would pass both tests above.
    ///
    /// Read this for what it is: the fixture hand-builds a frame whose
    /// `filename` is a repo-relative path, and a real `sentry-backtrace`
    /// frame carries a basename there (see `scrub_stacktrace`'s doc). What is
    /// pinned is that the scrubber does not eat a relative source path, which
    /// is a property of the scrubber. It is **not** evidence about the shape
    /// the SDK sends, and `culprit` is a field `sentry-panic` never sets at
    /// all.
    #[test]
    fn the_panic_itself_survives_the_scrubber() {
        let scrubbed = scrub_event(fully_populated_event()).expect("the event is never dropped");

        let exception = scrubbed.exception.values.first().expect("the exception");
        assert_eq!(exception.ty, "panic");
        assert_eq!(
            exception.value.as_deref(),
            Some("called `Option::unwrap()` on a `None` value")
        );

        let frame = exception
            .stacktrace
            .as_ref()
            .expect("a stacktrace")
            .frames
            .first()
            .expect("a frame");
        assert_eq!(frame.function.as_deref(), Some("outl_core::tree::apply_op"));
        assert_eq!(
            frame.filename.as_deref(),
            Some("crates/outl-core/src/tree/mod.rs"),
            "the compile-time, repo-relative panic location is the point of the report"
        );
        assert_eq!(frame.lineno, Some(42));
        assert_eq!(frame.abs_path.as_deref(), Some(crate::PATH_MARKER));

        assert_eq!(
            scrubbed.culprit.as_deref(),
            Some("crates/outl-core/src/tree/mod.rs:42")
        );
        assert_eq!(scrubbed.environment.as_deref(), Some("production"));
    }

    /// A debug image keeps the id the symbols are matched by and loses the
    /// path that names the user's home directory.
    #[test]
    fn a_debug_image_keeps_its_id_and_loses_its_path() {
        let scrubbed = scrub_event(fully_populated_event()).expect("the event is never dropped");

        let DebugImage::Symbolic(image) = scrubbed.debug_meta.images.first().expect("an image")
        else {
            panic!("expected the symbolic image the fixture builds");
        };
        assert_eq!(image.name, crate::PATH_MARKER);
        assert_eq!(image.id, DebugId::default());
    }

    /// The other half of RFC §3's promise. The three places that tell the user
    /// what a report carries — `CHANGELOG.md`, the RFC, and the GUI copy in
    /// `@outl/shared/onboarding` — all say the OS is in it, and without this the
    /// event had no producer for it: the `contexts` integration that would
    /// normally fill it is deliberately not linked.
    #[test]
    fn the_scrubber_writes_the_os_and_the_arch() {
        let event = scrub_event(fully_populated_event()).expect("never dropped");

        let Some(Context::Os(os)) = event.contexts.get("os") else {
            panic!("the scrubber must write an os context");
        };
        assert_eq!(os.name.as_deref(), Some(std::env::consts::OS));
        // Everything else about the OS stays absent: a kernel version and a
        // build number describe the user's machine, not the crash.
        assert_eq!(os.version, None);
        assert_eq!(os.kernel_version, None);
        assert_eq!(os.build, None);

        let Some(Context::Device(device)) = event.contexts.get("device") else {
            panic!("the scrubber must write a device context");
        };
        assert_eq!(device.arch.as_deref(), Some(std::env::consts::ARCH));
        assert_eq!(device.name, None);
        assert_eq!(device.model, None);
    }

    /// The direction that makes the write safe: an incoming `os` context is
    /// *replaced*, not merged into. The fixture arrives with a path sitting in
    /// `os.name`, which is what the `contexts` integration would look like if
    /// somebody enabled it, and none of it may survive.
    ///
    /// Without this, "the scrubber writes the OS" would also pass against an
    /// implementation that wrote the OS and left the rest of the map alone.
    #[test]
    fn an_incoming_os_context_is_replaced_not_merged() {
        let before = fully_populated_event();
        let Some(Context::Other(incoming)) = before.contexts.get("os") else {
            panic!("the fixture must populate os as an attacker-shaped Other");
        };
        assert_eq!(incoming.get("name"), Some(&json!(LEAK)));

        let event = scrub_event(fully_populated_event()).expect("never dropped");

        assert_eq!(event.contexts.len(), 2, "only os and device may be present");
        let payload = serde_json::to_string(&event).expect("serializes");
        assert!(
            !payload.contains(LEAK),
            "the incoming os context leaked into the payload"
        );
    }

    fn fully_populated_event() -> Event<'static> {
        let stacktrace = Stacktrace {
            frames: vec![Frame {
                function: Some("outl_core::tree::apply_op".into()),
                symbol: Some("_ZN9outl_core4tree8apply_op".into()),
                module: Some("outl_core::tree".into()),
                package: Some("outl-core".into()),
                filename: Some("crates/outl-core/src/tree/mod.rs".into()),
                abs_path: Some(LEAK.into()),
                lineno: Some(42),
                colno: Some(17),
                pre_context: vec![format!("let page = \"{LEAK}\";")],
                context_line: Some(format!("open(\"{LEAK}\")")),
                post_context: vec![format!("// {LEAK}")],
                in_app: Some(true),
                vars: [("slug".to_string(), json!(LEAK))].into_iter().collect(),
                image_addr: Some(Addr(0x1000)),
                instruction_addr: Some(Addr(0x1042)),
                ..Default::default()
            }],
            ..Default::default()
        };

        Event {
            event_id: uuid_nil_replacement(),
            level: Level::Fatal,
            fingerprint: Cow::Owned(vec![Cow::Owned(LEAK.to_string())]),
            culprit: Some("crates/outl-core/src/tree/mod.rs:42".into()),
            transaction: Some(format!("open {LEAK}")),
            message: Some(format!("could not read {LEAK}")),
            logentry: Some(LogEntry {
                message: format!("could not read {LEAK}"),
                params: vec![json!(LEAK)],
            }),
            logger: Some("outl_core::tree".into()),
            modules: [("outl-core".to_string(), LEAK.to_string())]
                .into_iter()
                .collect(),
            platform: Cow::Borrowed("native"),
            timestamp: SystemTime::UNIX_EPOCH,
            server_name: Some(Cow::Borrowed("avelino-macbook.local")),
            release: Some(Cow::Borrowed("cli@0.12.0")),
            dist: Some(Cow::Borrowed("1")),
            environment: Some(Cow::Borrowed("production")),
            user: Some(User {
                id: Some("avelino".into()),
                email: Some("avelino@example.com".into()),
                username: Some("avelino".into()),
                ..Default::default()
            }),
            request: Some(Request {
                query_string: Some(format!("page={LEAK}")),
                ..Default::default()
            }),
            contexts: [(
                "os".to_string(),
                Context::Other([("name".to_string(), json!(LEAK))].into_iter().collect()),
            )]
            .into_iter()
            .collect(),
            breadcrumbs: vec![Breadcrumb {
                message: Some(format!("read {LEAK}")),
                ..Default::default()
            }]
            .into(),
            exception: vec![Exception {
                ty: "panic".into(),
                value: Some("called `Option::unwrap()` on a `None` value".into()),
                module: Some("outl_core::tree".into()),
                stacktrace: Some(stacktrace.clone()),
                raw_stacktrace: Some(stacktrace.clone()),
                thread_id: None,
                mechanism: Some(Mechanism {
                    ty: "panic".into(),
                    description: Some(format!("while reading {LEAK}")),
                    data: [("path".to_string(), json!(LEAK))].into_iter().collect(),
                    ..Default::default()
                }),
            }]
            .into(),
            stacktrace: Some(stacktrace.clone()),
            template: Some(TemplateInfo {
                filename: Some(LEAK.into()),
                abs_path: Some(LEAK.into()),
                ..Default::default()
            }),
            threads: vec![Thread {
                name: Some(LEAK.into()),
                stacktrace: Some(stacktrace),
                ..Default::default()
            }]
            .into(),
            tags: [("workspace".to_string(), LEAK.to_string())]
                .into_iter()
                .collect(),
            extra: [("slug".to_string(), json!(LEAK))].into_iter().collect(),
            debug_meta: Cow::Owned(DebugMeta {
                sdk_info: Some(SystemSdkInfo {
                    sdk_name: "macosx".into(),
                    version_major: 15,
                    version_minor: 0,
                    version_patchlevel: 0,
                }),
                images: vec![DebugImage::Symbolic(SymbolicDebugImage {
                    name: LEAK.to_string(),
                    arch: Some("arm64".into()),
                    image_addr: Addr(0x1000),
                    image_size: 0x2000,
                    image_vmaddr: Addr(0),
                    id: DebugId::default(),
                    code_id: None,
                    debug_file: Some(LEAK.to_string()),
                })],
            }),
            sdk: Some(Cow::Owned(ClientSdkInfo {
                name: "sentry.rust".into(),
                version: "0.49.3".into(),
                integrations: vec!["panic".into()],
                packages: vec![],
            })),
        }
    }

    /// `Event::default()` leaves `event_id` nil and the client fills it in
    /// later; the fixture needs a concrete one so the key is serialized.
    fn uuid_nil_replacement() -> Uuid {
        Uuid::from_u128(0x0123_4567_89ab_cdef_0123_4567_89ab_cdef)
    }

    /// Guards the guard: if `serde_json` ever stops serializing a populated
    /// field the fixture sets, the key-set test would pass for the wrong
    /// reason.
    #[test]
    fn the_fixture_really_populates_everything() {
        let json: Value = serde_json::to_value(fully_populated_event()).expect("serialize");
        let keys = json.as_object().expect("object").len();
        assert!(
            keys >= 25,
            "the fixture should serialize nearly every protocol field, got {keys} keys: {json:#}"
        );
    }
}
