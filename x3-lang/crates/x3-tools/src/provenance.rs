//! Source provenance for an artifact — spec PHASE 47.
//!
//! The phase asks for "source hash, repository commit, build environment,
//! compiler binary hash, dependency lock hash, artifact hash" so that the system
//! can answer *which exact source produced this exact strategy artifact*. This
//! module records those facts when `x3c build` writes an artifact.
//!
//! Two rules it follows, because a provenance record that guesses is worse than
//! none:
//!
//! - every field is either **measured** (a hash of bytes that exist, the
//!   compiler's own version, the running binary's digest) or the string
//!   `"unknown"` with the reason in `notes`. A released binary built outside a
//!   repository has no commit; it says so rather than inventing one;
//! - the document names what each hash is *of*, because "source hash" is
//!   ambiguous the moment a file includes another.

use std::path::{Path, PathBuf};

use sha2::{Digest, Sha256};

/// What produced an artifact, as far as the build could observe.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Provenance {
    /// `sha256` of the source text exactly as it was read.
    pub source_hash: String,
    /// The path the source was read from, as given on the command line.
    pub source_path: String,
    /// `sha256` of the artifact bytes this document is about.
    pub artifact_hash: String,
    /// The artifact's path, as given on the command line.
    pub artifact_path: String,
    /// The compiler's crate version.
    pub compiler_version: String,
    /// `sha256` of the `x3c` binary that ran, or `"unknown"` when the running
    /// executable cannot be read.
    pub compiler_binary_hash: String,
    /// The repository commit, when the build ran inside a checkout that `git` can
    /// answer for; `"unknown"` otherwise.
    pub repository_commit: String,
    /// The `Cargo.lock` that was hashed, and its digest, when one was found beside
    /// the source; `"unknown"` otherwise.
    pub dependency_lock: String,
    pub dependency_lock_hash: String,
    /// Facts about the environment the build ran in — observed, not assumed.
    pub build_environment: Vec<(String, String)>,
    /// Anything that could not be measured, and why.
    pub notes: Vec<String>,
}

impl Provenance {
    /// The document as JSON, with a stable key order (determinism, PHASE 42).
    pub fn to_json(&self) -> String {
        let mut out = String::from("{\n");
        let fields: Vec<(&str, &str)> = vec![
            ("source_path", self.source_path.as_str()),
            ("source_hash", self.source_hash.as_str()),
            ("artifact_path", self.artifact_path.as_str()),
            ("artifact_hash", self.artifact_hash.as_str()),
            ("compiler_version", self.compiler_version.as_str()),
            ("compiler_binary_hash", self.compiler_binary_hash.as_str()),
            ("repository_commit", self.repository_commit.as_str()),
            ("dependency_lock", self.dependency_lock.as_str()),
            ("dependency_lock_hash", self.dependency_lock_hash.as_str()),
        ];
        for (index, (key, value)) in fields.iter().enumerate() {
            out.push_str(&format!("  \"{key}\": \"{value}\""));
            if index + 1 != fields.len() || !self.build_environment.is_empty() || !self.notes.is_empty() {
                out.push(',');
            }
            out.push('\n');
        }
        if !self.build_environment.is_empty() {
            out.push_str("  \"build_environment\": {\n");
            for (index, (key, value)) in self.build_environment.iter().enumerate() {
                out.push_str(&format!("    \"{key}\": \"{value}\""));
                if index + 1 != self.build_environment.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("  }");
            if !self.notes.is_empty() {
                out.push(',');
            }
            out.push('\n');
        }
        if !self.notes.is_empty() {
            out.push_str("  \"notes\": [\n");
            for (index, note) in self.notes.iter().enumerate() {
                out.push_str(&format!("    \"{note}\""));
                if index + 1 != self.notes.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("  ]\n");
        }
        out.push_str("}\n");
        out
    }
}

/// `sha256:<hex>` — the prefix names the algorithm, so a consumer never has to
/// guess which digest it is holding.
pub fn digest(bytes: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(bytes);
    format!("sha256:{}", hex(&hasher.finalize()))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// Observe what can be observed about this build.
///
/// `git_commit` is passed in rather than shelled out to here: the CLI is the layer
/// that may run a process, and a library that silently shells out is a library
/// that fails differently under test.
pub fn observe(
    source_path: &Path,
    source: &str,
    artifact_path: &Path,
    artifact: &[u8],
    git_commit: Option<String>,
) -> Provenance {
    let mut notes = Vec::new();

    let compiler_binary_hash = match std::env::current_exe().ok().and_then(|path| std::fs::read(path).ok()) {
        Some(bytes) => digest(&bytes),
        None => {
            notes.push("the running executable could not be read, so its digest is not recorded".to_string());
            "unknown".to_string()
        }
    };

    let repository_commit = match git_commit {
        Some(commit) if !commit.trim().is_empty() => commit.trim().to_string(),
        _ => {
            notes.push(
                "no repository commit was observable (the build did not run inside a git checkout that \
                 answered, or git is absent); the field is not guessed"
                    .to_string(),
            );
            "unknown".to_string()
        }
    };

    // The lockfile that matters is the one the compiler was built against, and the
    // nearest `Cargo.lock` above the *source* is the best available statement of
    // it: recorded with its path so a reader can see which lock was hashed.
    let (dependency_lock, dependency_lock_hash) = match nearest_lockfile(source_path) {
        Some(path) => match std::fs::read(&path) {
            Ok(bytes) => (path.display().to_string(), digest(&bytes)),
            Err(error) => {
                notes.push(format!("found {} but could not read it: {error}", path.display()));
                ("unknown".to_string(), "unknown".to_string())
            }
        },
        None => {
            notes.push("no Cargo.lock was found beside the source or above it".to_string());
            ("unknown".to_string(), "unknown".to_string())
        }
    };

    let build_environment = vec![
        // The compiler's own version is a compile-time fact about this binary.
        ("compiler_package".to_string(), env!("CARGO_PKG_NAME").to_string()),
        (
            "compiler_crate_version".to_string(),
            env!("CARGO_PKG_VERSION").to_string(),
        ),
        // The environment the *running binary* was built for, which is what
        // `std::env::consts` reports; labelled as such rather than as "the build
        // machine", which this process cannot see.
        ("target_os".to_string(), std::env::consts::OS.to_string()),
        ("target_arch".to_string(), std::env::consts::ARCH.to_string()),
        ("debug_assertions".to_string(), cfg!(debug_assertions).to_string()),
    ];

    Provenance {
        source_hash: digest(source.as_bytes()),
        source_path: source_path.display().to_string(),
        artifact_hash: digest(artifact),
        artifact_path: artifact_path.display().to_string(),
        compiler_version: env!("CARGO_PKG_VERSION").to_string(),
        compiler_binary_hash,
        repository_commit,
        dependency_lock,
        dependency_lock_hash,
        build_environment,
        notes,
    }
}

/// The nearest `Cargo.lock` at or above the source file's directory.
fn nearest_lockfile(source_path: &Path) -> Option<PathBuf> {
    let mut directory = source_path.parent()?.to_path_buf();
    for _ in 0..6 {
        let candidate = directory.join("Cargo.lock");
        if candidate.is_file() {
            return Some(candidate);
        }
        directory = directory.parent()?.to_path_buf();
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_digest_names_its_algorithm() {
        // The empty string's SHA-256, which is a value nobody has to recompute to
        // check that the encoding is right.
        assert_eq!(
            digest(b""),
            "sha256:e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
        );
    }

    #[test]
    fn the_document_is_stable_and_says_what_it_could_not_measure() {
        let source = "intent a { }";
        let provenance = observe(
            Path::new("/tmp/x3/prog.x3"),
            source,
            Path::new("/tmp/x3/prog.x3b"),
            &[1, 2, 3, 4],
            None,
        );
        let json = provenance.to_json();
        for expected in [
            "\"source_hash\": \"sha256:",
            "\"artifact_hash\": \"sha256:",
            "\"repository_commit\": \"unknown\"",
            "\"dependency_lock_hash\": \"",
            "\"compiler_version\": \"",
        ] {
            assert!(json.contains(expected), "missing {expected} in {json}");
        }
        assert!(
            json.contains("not guessed") || json.contains("no repository commit was observable"),
            "a field it could not measure must say so: {json}"
        );
        // Deterministic: the same inputs give the same bytes.
        let again = observe(
            Path::new("/tmp/x3/prog.x3"),
            source,
            Path::new("/tmp/x3/prog.x3b"),
            &[1, 2, 3, 4],
            None,
        );
        assert_eq!(provenance.source_hash, again.source_hash);
        assert_eq!(provenance.artifact_hash, again.artifact_hash);
    }

    #[test]
    fn a_commit_is_recorded_when_one_was_observed() {
        let provenance = observe(
            Path::new("/tmp/x3/prog.x3"),
            "source",
            Path::new("/tmp/x3/prog.x3b"),
            &[9],
            Some("abcdef1234\n".to_string()),
        );
        assert!(provenance.repository_commit == "abcdef1234", "{provenance:?}");
        assert!(
            provenance.notes.iter().all(|note| !note.contains("repository commit")),
            "and no note claims otherwise: {:?}",
            provenance.notes
        );
    }
}
