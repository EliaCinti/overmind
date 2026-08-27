//! Files moving in and out of an agent (M17).
//!
//! Overmind is deliberately **format-agnostic**: it does not parse PDFs,
//! convert spreadsheets or extract text from images. It puts the file in front
//! of the agent, tells it what the file is, and lets the adapter's own tools do
//! the reading — they are better at it than anything we would write here, and
//! they improve without us. What this module owns is the boring, easy-to-get-
//! wrong part: a safe name, an honest content type, a human size, and bytes
//! that survive the working directory being deleted.

use std::path::{Path, PathBuf};

/// A path-free, filesystem-safe basename. An uploaded name is attacker-shaped
/// input: it can carry `../`, a leading `.`, a null byte, or a Windows path.
pub fn safe_name(filename: &str) -> String {
    let base = filename.rsplit(['/', '\\']).next().unwrap_or(filename);
    let cleaned: String = base
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_') {
                c
            } else {
                '_'
            }
        })
        .collect();
    let cleaned = cleaned.trim_matches('.').to_string();
    if cleaned.is_empty() {
        "file".to_string()
    } else {
        cleaned
    }
}

/// The same, for a path *relative to a run directory* — each component is made
/// safe but the shape is kept, because `research/sources.csv` and
/// `sources.csv` are different deliverables and flattening them loses the
/// structure the agent chose.
pub fn safe_relative(path: &Path) -> String {
    path.components()
        .filter_map(|c| match c {
            std::path::Component::Normal(s) => Some(safe_name(&s.to_string_lossy())),
            // `..`, `/`, and drive prefixes are dropped, not escaped.
            _ => None,
        })
        .collect::<Vec<_>>()
        .join("/")
}

/// Content type from the file extension.
///
/// Deliberately a table and not a crate: the list of things worth naming is
/// short, and being wrong here is visible (the browser downloads what it could
/// have shown, or shows what it should have downloaded). Anything unlisted is
/// `application/octet-stream`, which is the honest answer — "bytes, I don't
/// know" — and the download path handles it correctly.
pub fn mime_for(name: &str) -> &'static str {
    let ext = Path::new(name)
        .extension()
        .map(|e| e.to_string_lossy().to_lowercase())
        .unwrap_or_default();
    match ext.as_str() {
        // Documents the agent writes
        "md" | "markdown" => "text/markdown",
        "txt" | "log" => "text/plain",
        "csv" => "text/csv",
        "tsv" => "text/tab-separated-values",
        "json" => "application/json",
        "yaml" | "yml" => "application/yaml",
        "toml" => "application/toml",
        "xml" => "application/xml",
        "html" | "htm" => "text/html",
        "pdf" => "application/pdf",
        "docx" => "application/vnd.openxmlformats-officedocument.wordprocessingml.document",
        "xlsx" => "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet",
        "pptx" => "application/vnd.openxmlformats-officedocument.presentationml.presentation",
        "odt" => "application/vnd.oasis.opendocument.text",
        "ods" => "application/vnd.oasis.opendocument.spreadsheet",
        "epub" => "application/epub+zip",
        // Code — named individually so the UI can highlight and the browser
        // can show rather than download.
        "rs" => "text/x-rust",
        "py" => "text/x-python",
        "ts" | "tsx" => "text/typescript",
        "js" | "jsx" | "mjs" | "cjs" => "text/javascript",
        "go" => "text/x-go",
        "java" => "text/x-java",
        "c" | "h" => "text/x-c",
        "cpp" | "cc" | "hpp" => "text/x-c++",
        "cs" => "text/x-csharp",
        "rb" => "text/x-ruby",
        "php" => "text/x-php",
        "swift" => "text/x-swift",
        "kt" | "kts" => "text/x-kotlin",
        "sh" | "bash" | "zsh" => "text/x-shellscript",
        "sql" => "application/sql",
        "css" => "text/css",
        "scss" | "sass" => "text/x-scss",
        "diff" | "patch" => "text/x-diff",
        "ipynb" => "application/x-ipynb+json",
        "tex" => "text/x-tex",
        // Images
        "png" => "image/png",
        "jpg" | "jpeg" => "image/jpeg",
        "gif" => "image/gif",
        "webp" => "image/webp",
        "svg" => "image/svg+xml",
        "avif" => "image/avif",
        "heic" => "image/heic",
        "bmp" => "image/bmp",
        "ico" => "image/x-icon",
        // Audio / video — an agent given a recording should be able to hand
        // one back.
        "mp3" => "audio/mpeg",
        "wav" => "audio/wav",
        "m4a" => "audio/mp4",
        "ogg" | "oga" => "audio/ogg",
        "flac" => "audio/flac",
        "mp4" | "m4v" => "video/mp4",
        "webm" => "video/webm",
        "mov" => "video/quicktime",
        // Archives
        "zip" => "application/zip",
        "gz" | "tgz" => "application/gzip",
        "tar" => "application/x-tar",
        "7z" => "application/x-7z-compressed",
        _ => "application/octet-stream",
    }
}

/// Whether this content type is something you have to *look* at.
///
/// Drives the multimodal gate at checkout (ADR-0021): a task carrying one of
/// these may only go to an agent characterized for visual work. SVG is excluded
/// on purpose — it is markup, readable as text by an agent that cannot see.
pub fn is_visual(mime: &str) -> bool {
    mime.starts_with("image/") && mime != "image/svg+xml"
}

/// Whether this content type is text a human can read in place.
///
/// Drives one decision only: render it inline, or offer a download. The
/// `text/*` families plus the structured-data types that happen to be text.
pub fn is_texty(mime: &str) -> bool {
    mime.starts_with("text/")
        || matches!(
            mime,
            "application/json"
                | "application/yaml"
                | "application/toml"
                | "application/xml"
                | "application/sql"
                | "application/x-ipynb+json"
        )
}

/// A size a person can read: `2.4 MB`, not `2516582`.
///
/// Used in prompts, so an agent knows before opening whether a file is a note
/// or a corpus, and in the UI for the same reason.
pub fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    let b = bytes as f64;
    if bytes < 1024 {
        return format!("{bytes} B");
    }
    let (value, unit) = if b < KB * KB {
        (b / KB, "KB")
    } else if b < KB * KB * KB {
        (b / (KB * KB), "MB")
    } else {
        (b / (KB * KB * KB), "GB")
    };
    // One decimal below 10 (2.4 MB), none above (24 MB) — the extra digit
    // stops mattering exactly where the number gets long.
    if value < 10.0 {
        format!("{value:.1} {unit}")
    } else {
        format!("{value:.0} {unit}")
    }
}

/// Every file under `root`, depth-first, as paths relative to it.
///
/// Recursive on purpose: a top-level-only scan silently dropped whatever an
/// agent organised into folders, which is exactly what an agent does when it
/// produces more than one thing. Hidden entries and `.git` are skipped — they
/// are machinery, not deliverables. Bounded so a runaway run cannot fill the
/// database with a million rows.
pub async fn collect_files(root: &Path, limit: usize) -> Vec<(PathBuf, u64)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(mut entries) = tokio::fs::read_dir(&dir).await else {
            continue;
        };
        // Deterministic order (name-sorted per directory): which files make
        // the cap must not depend on filesystem hash order — a produced file
        // either always rides or a test catches that it cannot.
        let mut batch = Vec::new();
        while let Ok(Some(entry)) = entries.next_entry().await {
            batch.push(entry);
        }
        batch.sort_by_key(|e| e.file_name());
        for entry in batch {
            if out.len() >= limit {
                return out;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            if name.starts_with('.') {
                continue;
            }
            let Ok(file_type) = entry.file_type().await else {
                continue;
            };
            if file_type.is_dir() {
                stack.push(entry.path());
            } else if file_type.is_file() {
                let size = entry.metadata().await.map(|m| m.len()).unwrap_or(0);
                if let Ok(rel) = entry.path().strip_prefix(root) {
                    out.push((rel.to_path_buf(), size));
                }
            }
        }
    }
    out.sort();
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_relative_path_keeps_its_shape_but_cannot_escape() {
        assert_eq!(
            safe_relative(Path::new("research/sources.csv")),
            "research/sources.csv"
        );
        assert_eq!(safe_relative(Path::new("../../etc/passwd")), "etc/passwd");
        assert_eq!(safe_relative(Path::new("/etc/passwd")), "etc/passwd");
    }

    #[test]
    fn the_content_type_is_specific_enough_to_render_by() {
        assert_eq!(mime_for("report.pdf"), "application/pdf");
        assert_eq!(mime_for("chart.PNG"), "image/png");
        assert_eq!(mime_for("solver.py"), "text/x-python");
        assert!(is_texty(mime_for("data.json")));
        assert!(!is_texty(mime_for("archive.zip")));
        // Unknown is admitted, not guessed.
        assert_eq!(mime_for("thing.qqq"), "application/octet-stream");
    }

    #[test]
    fn sizes_read_like_a_person_wrote_them() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2_516_582), "2.4 MB");
        assert_eq!(human_size(25_165_824), "24 MB");
    }
}
