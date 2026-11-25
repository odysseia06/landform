use std::fmt;
use std::path::PathBuf;

/// Every failure the library reports; each message is one readable sentence.
#[derive(Debug, Clone, PartialEq)]
pub enum LandformError {
    NoSources,
    SourceTooSmall { source_id: String, needed_m: f64 },
    OutOfBounds,
    NoUsableWindow { tries: u32 },
    PackNotFound(PathBuf),
    PackCorrupt { path: PathBuf, reason: String },
    Io(String),
    /// A generated height is `NaN` or infinite.
    NotFinite { source_id: String },
    /// A setting is out of range; the text names the field.
    BadConfig(String),
    /// The caller set the cancel flag while the world was being generated.
    Cancelled,
    /// An edit of a land refused because water it changes would lie beside ground that must not
    /// change (the editor tool's design, amendment 10): one sentence naming the river or lake, the
    /// place of that ground and what to do.
    Refused(String),
    /// A land file (`Land::read`) that is not a land this Landform reads: cut short, damaged, of
    /// another format, or holding a land whose parts do not fit together; `reason` says which, in
    /// words.
    BadLand { path: PathBuf, reason: String },
}

impl fmt::Display for LandformError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoSources => write!(f, "No terrain sources were given."),
            Self::SourceTooSmall { source_id, needed_m } => write!(
                f,
                "Source {source_id} is too small: it must cover at least {needed_m} metres."
            ),
            Self::OutOfBounds => write!(f, "The requested area lies outside the world."),
            Self::NoUsableWindow { tries } => {
                write!(f, "No usable window was found after {tries} tries.")
            }
            Self::PackNotFound(path) => {
                write!(f, "The pack file {} does not exist.", path.display())
            }
            Self::PackCorrupt { path, reason } => {
                write!(f, "The pack file {} is corrupt: {reason}.", path.display())
            }
            Self::Io(message) => write!(f, "An input or output error occurred: {message}."),
            Self::NotFinite { source_id } => write!(
                f,
                "The heights generated from source {source_id} contain a value that is not a number."
            ),
            Self::BadConfig(reason) => write!(f, "The generation settings are not valid: {reason}."),
            Self::Cancelled => write!(f, "The generation was cancelled."),
            Self::Refused(sentence) => write!(f, "{sentence}"),
            Self::BadLand { path, reason } => {
                write!(f, "The land file {} cannot be read: {reason}.", path.display())
            }
        }
    }
}

impl std::error::Error for LandformError {}

/// Whether `message` reads as one sentence, as every message of `LandformError` must: not empty, on
/// one line, ending in a full stop, with no full stop before its end followed by a space (where a
/// second sentence would begin). The land's tests ask it of every refusal sentence an edit builds.
#[cfg(test)]
pub(crate) fn is_one_sentence(message: &str) -> bool {
    !message.is_empty() && !message.contains('\n') && message.ends_with('.') && !message.contains(". ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn pack_messages_name_the_pack_and_path() {
        let missing = LandformError::PackNotFound(PathBuf::from("tiles/a.pack")).to_string();
        assert!(missing.contains("pack") && missing.contains("a.pack"));
        let corrupt = LandformError::PackCorrupt {
            path: PathBuf::from("tiles/b.pack"),
            reason: "bad header".to_string(),
        }
        .to_string();
        assert!(corrupt.contains("pack") && corrupt.contains("b.pack") && corrupt.contains("bad header"));
    }

    #[test]
    fn every_message_is_one_sentence() {
        let all = [
            LandformError::NoSources,
            LandformError::SourceTooSmall { source_id: "s1".into(), needed_m: 1500.5 },
            LandformError::OutOfBounds,
            LandformError::NoUsableWindow { tries: 8 },
            LandformError::PackNotFound(PathBuf::from("p.pack")),
            LandformError::PackCorrupt { path: PathBuf::from("p.pack"), reason: "x".into() },
            LandformError::Io("disk full".into()),
            LandformError::NotFinite { source_id: "s1".into() },
            LandformError::BadConfig("detail_amplitude_m must not be negative".into()),
            LandformError::Cancelled,
            LandformError::Refused("River 2's new line runs within 150 m of held ground at (-5, 426), 124 m from it; the water there would not be held: move the line, or lift the hold there.".into()),
            LandformError::BadLand { path: PathBuf::from("valley.lfland"), reason: "it holds no MARK section (the marks), so it may have been cut short".into() },
        ];
        for e in all {
            let m = e.to_string();
            assert!(is_one_sentence(&m), "{m}");
        }
    }

    #[test]
    fn not_finite_names_the_source_and_bad_config_names_the_field() {
        let not_finite = LandformError::NotFinite { source_id: "olympic".into() }.to_string();
        assert!(not_finite.contains("olympic") && not_finite.contains("not a number"), "{not_finite}");
        let bad = LandformError::BadConfig("detail_amplitude_m must not be negative".into()).to_string();
        assert!(bad.contains("detail_amplitude_m"), "{bad}");
    }

    #[test]
    fn a_land_file_error_names_the_file_and_the_fault() {
        let bad = LandformError::BadLand { path: PathBuf::from("lands/valley.lfland"), reason: "it was written in format 2, and this Landform reads format 1".into() }
            .to_string();
        assert_eq!(bad, "The land file lands/valley.lfland cannot be read: it was written in format 2, and this Landform reads format 1.");
    }

    #[test]
    fn a_refused_edit_says_its_own_sentence() {
        let sentence = "Lake 1 is held at (-80, -160), so the edit cannot move, resize or take it away; lift the hold on it first.";
        assert_eq!(LandformError::Refused(sentence.into()).to_string(), sentence);
    }
}
