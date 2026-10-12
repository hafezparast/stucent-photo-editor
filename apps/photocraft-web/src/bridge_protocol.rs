//! The host bridge's rules, apart from the browser glue in `host_bridge.rs` so they are tested
//! natively: which page may drive the app, which messages exist, what exam mode locks, how often
//! the app sends autosave snapshots.

// The constants are read by the browser glue only; native builds compile this file for its tests.
#![cfg_attr(not(target_arch = "wasm32"), allow(dead_code))]

/// The protocol name every message carries in its `bridge` field.
pub const PROTOCOL: &str = "emu/1";
/// This app, as named in the `ready` handshake.
pub const APP: &str = "stucent-photo-editor";
/// The release tag of this build of the fork (`ready.version`); the release workflow checks that
/// it matches the tag it builds.
pub const VERSION: &str = "v0.7.0-stucent.1";
/// The upstream PhotoCraft version the fork is based on (`ready.upstream`).
pub const UPSTREAM: &str = env!("CARGO_PKG_VERSION");
/// What the host can ask for (`ready.capabilities`).
pub const CAPABILITIES: &[&str] = &["open", "export.psd", "export.png", "exam", "autosave", "commands", "lang"];
/// Longest `params` JSON forwarded per command in the edit log (brush strokes carry every point).
pub const MAX_PARAMS_CHARS: usize = 512;
/// Autosave never runs more often than this (a PSD export takes a few hundred milliseconds).
pub const MIN_AUTOSAVE_MS: u32 = 5_000;
/// Nor less often than this while it is on.
pub const MAX_AUTOSAVE_MS: u32 = 600_000;

/// A host message the app understands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// `hello` or `ping`: answered with `ready`.
    Hello,
    Open,
    Export,
    Exam,
    Autosave,
}

/// What `export` writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Psd,
    /// The flattened composite.
    Png,
}

impl Format {
    pub fn name(self) -> &'static str {
        match self {
            Format::Psd => "psd",
            Format::Png => "png",
        }
    }

    /// The file name handed to the exporter (its extension picks the format).
    pub fn file(self) -> &'static str {
        match self {
            Format::Psd => "submission.psd",
            Format::Png => "submission.png",
        }
    }
}

/// The host page's origin from the `?host=` parameter: `http(s)://host[:port]` only (no path,
/// query, user name or wildcard), with a trailing `/` dropped. `None` leaves the bridge off.
pub fn host_origin(param: Option<&str>) -> Option<String> {
    let raw = param?.trim();
    let raw = raw.strip_suffix('/').unwrap_or(raw);
    let (scheme, rest) = raw.split_once("://")?;
    if !matches!(scheme, "http" | "https") || rest.is_empty() {
        return None;
    }
    // The authority only: a host name or IP literal, then an optional port.
    let ok = rest.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | ':' | '[' | ']'));
    if !ok {
        return None;
    }
    if let Some((host, port)) = rest.rsplit_once(':')
        && !host.ends_with(']')
        && !host.contains(':')
        && (host.is_empty() || port.is_empty() || !port.chars().all(|c| c.is_ascii_digit()))
    {
        return None;
    }
    Some(format!("{scheme}://{rest}"))
}

/// Is a message from `origin`, sent by `from_parent`, one the bridge may act on?
pub fn accepts(expected: &str, origin: &str, from_parent: bool) -> bool {
    from_parent && !expected.is_empty() && origin == expected
}

/// The message kind from its `bridge` and `type` fields; `None` for anything else.
pub fn kind(bridge: Option<&str>, ty: Option<&str>) -> Option<Kind> {
    if bridge != Some(PROTOCOL) {
        return None;
    }
    match ty? {
        "hello" | "ping" => Some(Kind::Hello),
        "open" => Some(Kind::Open),
        "export" => Some(Kind::Export),
        "exam" => Some(Kind::Exam),
        "autosave" => Some(Kind::Autosave),
        _ => None,
    }
}

/// The format an `export` asks for (`psd` when not given); `None` for an unknown one.
pub fn export_format(format: Option<&str>) -> Option<Format> {
    match format.map(str::to_ascii_lowercase).as_deref() {
        None | Some("psd") => Some(Format::Psd),
        Some("png") => Some(Format::Png),
        Some(_) => None,
    }
}

/// The autosave interval for `autosave {intervalMs}`: `None` turns autosave off (0, negative,
/// missing or not a number); otherwise clamped to [`MIN_AUTOSAVE_MS`]..=[`MAX_AUTOSAVE_MS`].
pub fn autosave_interval(ms: Option<f64>) -> Option<u32> {
    let ms = ms.filter(|m| m.is_finite() && *m > 0.0)?;
    Some(ms.clamp(f64::from(MIN_AUTOSAVE_MS), f64::from(MAX_AUTOSAVE_MS)) as u32)
}

/// Is a snapshot due? Only when the interval has passed and commands ran since the last one.
pub fn snapshot_due(now_ms: f64, due_ms: f64, commands: u64, saved_at: u64) -> bool {
    now_ms >= due_ms && commands > saved_at
}

/// Commands exam mode refuses: anything that brings outside content in (files, other apps'
/// clipboards are not wired on the web) or takes the work out other than through the host
/// (save, export, print). The internal clipboard (copy, paste between layers) stays.
pub fn exam_blocks(id: &str) -> bool {
    const PREFIXES: &[&str] = &[
        // In: files, folders and recorded batches that read them.
        "file.open",
        "file.place",
        "file.import.",
        "file.automate.",
        "file.recover",
        "file.revert",
        "file.exrParts",
        // Out: every way to write a file but the host bridge's own export.
        "file.save",
        "file.export.",
        "file.package",
        "file.print",
        "file.generate.",
        "edit.presets.",
    ];
    const IDS: &[&str] = &["file.scripts.loadFilesIntoStack", "file.scripts.browse", "file.scripts.imageProcessor", "file.revealInFinder"];
    PREFIXES.iter().any(|p| id.starts_with(p)) || IDS.contains(&id)
}

/// A command's params as forwarded in the edit log, cut to [`MAX_PARAMS_CHARS`] characters.
pub fn truncate_params(text: &str) -> String {
    if text.chars().count() <= MAX_PARAMS_CHARS {
        return text.to_string();
    }
    let mut out: String = text.chars().take(MAX_PARAMS_CHARS).collect();
    out.push('…');
    out
}

/// The UI language code for `?lang=` (a StuCent locale such as `pt`, `zh` or `ja`): a registered
/// language, or English when the app has none for it (ar, fa and ta wait for RTL and fonts).
pub fn ui_language(code: &str) -> &'static str {
    photocraft_ui_egui::i18n::lang_from_tag(code).map_or("en", photocraft_ui_egui::i18n::Lang::code)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn host_origin_accepts_plain_http_origins_only() {
        assert_eq!(host_origin(Some("https://stucent.com")).as_deref(), Some("https://stucent.com"));
        assert_eq!(host_origin(Some("http://localhost:5225")).as_deref(), Some("http://localhost:5225"));
        assert_eq!(host_origin(Some("https://dev-stucent.cubie.kidocode.com/")).as_deref(), Some("https://dev-stucent.cubie.kidocode.com"));
        for bad in ["", "*", "null", "javascript:alert(1)", "ftp://a.com", "https://a.com/path", "https://a.com?x=1", "https://a.com#x", "https://u@a.com", "https://", "https://a b.com", "data:text/html,x"] {
            assert_eq!(host_origin(Some(bad)), None, "{bad:?}");
        }
        assert_eq!(host_origin(None), None);
    }

    #[test]
    fn only_the_named_origin_in_the_parent_window_is_heard() {
        assert!(accepts("https://stucent.com", "https://stucent.com", true));
        assert!(!accepts("https://stucent.com", "https://stucent.com", false), "another window of the same origin");
        assert!(!accepts("https://stucent.com", "https://evil.example", true));
        assert!(!accepts("https://stucent.com", "https://stucent.com.evil.example", true));
        assert!(!accepts("https://stucent.com", "null", true));
    }

    #[test]
    fn messages_need_the_protocol_and_a_known_type() {
        assert_eq!(kind(Some("emu/1"), Some("hello")), Some(Kind::Hello));
        assert_eq!(kind(Some("emu/1"), Some("ping")), Some(Kind::Hello));
        assert_eq!(kind(Some("emu/1"), Some("open")), Some(Kind::Open));
        assert_eq!(kind(Some("emu/1"), Some("export")), Some(Kind::Export));
        assert_eq!(kind(Some("emu/1"), Some("exam")), Some(Kind::Exam));
        assert_eq!(kind(Some("emu/1"), Some("autosave")), Some(Kind::Autosave));
        assert_eq!(kind(Some("emu/2"), Some("open")), None, "another protocol version");
        assert_eq!(kind(None, Some("open")), None);
        assert_eq!(kind(Some("emu/1"), Some("eval")), None);
        assert_eq!(kind(Some("emu/1"), None), None);
    }

    #[test]
    fn export_formats() {
        assert_eq!(export_format(None), Some(Format::Psd));
        assert_eq!(export_format(Some("psd")), Some(Format::Psd));
        assert_eq!(export_format(Some("PNG")), Some(Format::Png));
        assert_eq!(export_format(Some("pcraft")), None);
        assert_eq!(Format::Png.name(), "png");
        assert_eq!(Format::Png.file(), "submission.png");
        assert_eq!(Format::Psd.file(), "submission.psd");
    }

    #[test]
    fn autosave_interval_is_clamped_and_zero_turns_it_off() {
        assert_eq!(autosave_interval(Some(0.0)), None);
        assert_eq!(autosave_interval(Some(-5.0)), None);
        assert_eq!(autosave_interval(Some(f64::NAN)), None);
        assert_eq!(autosave_interval(None), None);
        assert_eq!(autosave_interval(Some(1.0)), Some(MIN_AUTOSAVE_MS));
        assert_eq!(autosave_interval(Some(30_000.0)), Some(30_000));
        assert_eq!(autosave_interval(Some(1e12)), Some(MAX_AUTOSAVE_MS));
    }

    #[test]
    fn a_snapshot_is_sent_only_after_new_commands() {
        assert!(!snapshot_due(1_000.0, 5_000.0, 3, 0), "not yet due");
        assert!(snapshot_due(5_000.0, 5_000.0, 3, 0), "due, with new commands");
        assert!(!snapshot_due(9_000.0, 5_000.0, 3, 3), "due, but nothing changed since the last snapshot");
    }

    #[test]
    fn exam_mode_locks_files_in_and_out_but_not_editing() {
        for id in [
            "file.open",
            "file.openAs",
            "file.openRecent.0",
            "file.openExrParts",
            "file.exrParts",
            "file.placeEmbedded",
            "file.placeLinked",
            "file.import.videoFramesToLayers",
            "file.automate.batch",
            "file.scripts.loadFilesIntoStack",
            "file.scripts.browse",
            "file.scripts.imageProcessor",
            "file.revert",
            "file.recover",
            "file.save",
            "file.saveAs",
            "file.saveACopy",
            "file.export.exportAs",
            "file.export.quickExportAsPng",
            "file.export.layersToFiles",
            "file.package",
            "file.print",
            "file.printOneCopy",
            "file.generate.imageAssets",
            "edit.presets.exportImportPresets",
            "edit.presets.migratePresets",
        ] {
            assert!(exam_blocks(id), "{id} should be locked");
        }
        for id in [
            "file.new",
            "file.newFromClipboard",
            "file.close",
            "edit.copy",
            "edit.cut",
            "edit.copyMerged",
            "edit.paste",
            "edit.pasteSpecial.pasteInPlace",
            "edit.undo",
            "layer.new.layerViaCopy",
            "layer.setProps",
            "image.canvasSize",
            "image.crop",
            "type.create",
            "layer.newAdjustmentLayer.brightnessContrast",
        ] {
            assert!(!exam_blocks(id), "{id} should stay allowed");
        }
    }

    #[test]
    fn long_params_are_cut() {
        assert_eq!(truncate_params("{\"a\":1}"), "{\"a\":1}");
        let long = "x".repeat(2_000);
        let cut = truncate_params(&long);
        assert_eq!(cut.chars().count(), MAX_PARAMS_CHARS + 1);
        assert!(cut.ends_with('…'));
        // Multi-byte text is cut on a character boundary.
        let wide = "é".repeat(600);
        assert_eq!(truncate_params(&wide).chars().count(), MAX_PARAMS_CHARS + 1);
    }

    #[test]
    fn stucent_locales_map_to_ui_languages() {
        assert_eq!(ui_language("ja"), "ja");
        assert_eq!(ui_language("de"), "de");
        assert_eq!(ui_language("pt"), "pt-br");
        assert_eq!(ui_language("zh"), "zh-hans");
        assert_eq!(ui_language("en"), "en");
        for blocked in ["ar", "fa", "ta", "xx", ""] {
            assert_eq!(ui_language(blocked), "en", "{blocked}");
        }
    }
}
