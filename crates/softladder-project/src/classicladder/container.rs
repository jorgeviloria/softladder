//! Parser and serializer for the ClassicLadder text container.
//!
//! The container framing is line based and deliberately tolerant:
//!
//! * the first line must start with `_FILES_CLASSICLADDER`;
//! * each parameter file is introduced by `_FILE-<name>` and closed by
//!   `_/FILE-<name>`; the text between the two markers, including its line
//!   terminators, is the file content;
//! * the container ends with `_/FILES_CLASSICLADDER`.
//!
//! Blank lines outside a parameter file are ignored. Everything else outside a
//! parameter file, a mismatched closing marker or a missing end marker is a
//! [`ProjectError::Container`]. Files that begin with the gzip magic number are
//! decompressed first, which is how `.clprjz` projects are handled.

use std::io::Read;

use flate2::read::GzDecoder;

use crate::native::is_gzip;
use crate::ProjectError;

/// Marker that opens a ClassicLadder container.
pub const CONTAINER_START: &str = "_FILES_CLASSICLADDER";
/// Marker that closes a ClassicLadder container.
pub const CONTAINER_END: &str = "_/FILES_CLASSICLADDER";
/// Prefix that opens one parameter file inside the container.
pub const FILE_PREFIX: &str = "_FILE-";
/// Prefix that closes one parameter file inside the container.
pub const FILE_END_PREFIX: &str = "_/FILE-";

/// Decodes `bytes` as text, transparently decompressing gzip streams.
pub fn decode_maybe_gzip(bytes: &[u8]) -> Result<String, ProjectError> {
    let raw = if is_gzip(bytes) {
        let mut decoder = GzDecoder::new(bytes);
        let mut buffer = Vec::new();
        decoder.read_to_end(&mut buffer).map_err(|error| {
            ProjectError::Container(format!("cannot decompress project: {error}"))
        })?;
        buffer
    } else {
        bytes.to_vec()
    };
    String::from_utf8(raw)
        .map_err(|error| ProjectError::Container(format!("container is not valid UTF-8: {error}")))
}

/// Parses a ClassicLadder container into ordered `(name, content)` pairs.
///
/// The content of each part keeps its original line terminators, so
/// [`serialize_container`] reproduces the input byte for byte.
pub fn parse_container(bytes: &[u8]) -> Result<Vec<(String, String)>, ProjectError> {
    let text = decode_maybe_gzip(bytes)?;
    let mut lines = text.split_inclusive('\n');

    let header = lines
        .next()
        .ok_or_else(|| ProjectError::Container("empty container".to_owned()))?
        .trim_end_matches(['\r', '\n']);
    if !header.starts_with(CONTAINER_START) {
        return Err(ProjectError::Container(format!(
            "missing `{CONTAINER_START}` header"
        )));
    }

    let mut parts: Vec<(String, String)> = Vec::new();
    let mut current: Option<(String, String)> = None;
    let mut complete = false;

    for line in lines {
        let trimmed = line.trim_end_matches(['\r', '\n']);
        if trimmed.starts_with(CONTAINER_END) {
            if let Some((name, _)) = current.as_ref() {
                return Err(ProjectError::Container(format!(
                    "file part `{name}` is not closed before `{CONTAINER_END}`"
                )));
            }
            complete = true;
            break;
        }

        if let Some(name) = trimmed.strip_prefix(FILE_END_PREFIX) {
            let name = name.trim();
            match current.take() {
                Some((open, content)) if open == name => parts.push((open, content)),
                Some((open, _)) => {
                    return Err(ProjectError::Container(format!(
                        "file part `{open}` is closed by `{name}`"
                    )));
                }
                None => {
                    return Err(ProjectError::Container(format!(
                        "unexpected `{trimmed}` outside a file part"
                    )));
                }
            }
        } else if let Some(name) = trimmed.strip_prefix(FILE_PREFIX) {
            if let Some((open, _)) = current.as_ref() {
                return Err(ProjectError::Container(format!(
                    "file part `{open}` is not closed before `{name}`"
                )));
            }
            current = Some((name.trim().to_owned(), String::new()));
        } else if let Some((_, content)) = current.as_mut() {
            content.push_str(line);
        } else if !trimmed.trim().is_empty() {
            return Err(ProjectError::Container(format!(
                "content outside a file part: `{trimmed}`"
            )));
        }
    }

    if let Some((name, _)) = current.as_ref() {
        return Err(ProjectError::Container(format!(
            "file part `{name}` is never closed"
        )));
    }
    if !complete {
        return Err(ProjectError::Container(format!(
            "missing `{CONTAINER_END}` end marker"
        )));
    }
    Ok(parts)
}

/// Serializes `(name, content)` pairs back into a ClassicLadder container.
pub fn serialize_container(parts: &[(String, String)]) -> String {
    let mut out = String::new();
    out.push_str(CONTAINER_START);
    out.push('\n');
    for (name, content) in parts {
        out.push_str(FILE_PREFIX);
        out.push_str(name);
        out.push('\n');
        out.push_str(content);
        if !content.is_empty() && !content.ends_with('\n') {
            out.push('\n');
        }
        out.push_str(FILE_END_PREFIX);
        out.push_str(name);
        out.push('\n');
    }
    out.push_str(CONTAINER_END);
    out.push('\n');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    /// Small hand-written container in the shape ClassicLadder produces.
    const SAMPLE: &str = concat!(
        "_FILES_CLASSICLADDER\n",
        "_FILE-general.txt\n",
        "#VER=3.0\n",
        "SIZE_INFO_LADDER=300\n",
        "_/FILE-general.txt\n",
        "_FILE-rungs.txt\n",
        "#VER=3.0\n",
        "1,1,0,0,0,0,0,0\n",
        "_/FILE-rungs.txt\n",
        "_/FILES_CLASSICLADDER\n",
    );

    fn sample_parts() -> Vec<(String, String)> {
        vec![
            (
                "general.txt".to_owned(),
                "#VER=3.0\nSIZE_INFO_LADDER=300\n".to_owned(),
            ),
            (
                "rungs.txt".to_owned(),
                "#VER=3.0\n1,1,0,0,0,0,0,0\n".to_owned(),
            ),
        ]
    }

    #[test]
    fn sample_container_parses_into_ordered_parts() {
        let parts = parse_container(SAMPLE.as_bytes()).expect("container parses");
        assert_eq!(parts.len(), 2);
        assert_eq!(parts[0].0, "general.txt");
        assert!(parts[0].1.starts_with("#VER=3.0"));
        assert!(parts[0].1.contains("SIZE_INFO_LADDER=300"));
        assert_eq!(parts[1].0, "rungs.txt");
        assert!(parts[1].1.starts_with("#VER=3.0"));
        assert_eq!(parts, sample_parts());
    }

    #[test]
    fn serialization_round_trips() {
        let parts = sample_parts();
        let text = serialize_container(&parts);
        assert!(text.starts_with("_FILES_CLASSICLADDER\n"));
        assert!(text.ends_with("_/FILES_CLASSICLADDER\n"));
        assert_eq!(text, SAMPLE);
        assert_eq!(
            parse_container(text.as_bytes()).expect("round trip parses"),
            parts
        );
    }

    #[test]
    fn gzip_containers_are_transparently_decoded() {
        let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
        encoder
            .write_all(SAMPLE.as_bytes())
            .expect("compresses sample");
        let compressed = encoder.finish().expect("finishes stream");
        assert!(is_gzip(&compressed));
        assert_eq!(
            parse_container(&compressed).expect("gzip container parses"),
            sample_parts()
        );
    }

    #[test]
    fn content_without_a_trailing_newline_is_normalized() {
        let parts = vec![("small.txt".to_owned(), "#VER=3.0".to_owned())];
        let text = serialize_container(&parts);
        // The closing marker must start on its own line, so the serializer
        // terminates the last content line and the parser sees it terminated.
        assert_eq!(
            parse_container(text.as_bytes()).expect("parses"),
            vec![("small.txt".to_owned(), "#VER=3.0\n".to_owned())]
        );
        assert_eq!(
            serialize_container(&parse_container(text.as_bytes()).expect("parses")),
            text
        );
    }

    #[test]
    fn malformed_containers_are_rejected() {
        let cases = [
            "",
            "_FILE-general.txt\n#VER=3.0\n_/FILE-general.txt\n_/FILES_CLASSICLADDER\n",
            "_FILES_CLASSICLADDER\n_FILE-general.txt\n#VER=3.0\n",
            "_FILES_CLASSICLADDER\n_FILE-general.txt\n#VER=3.0\n_/FILE-other.txt\n_/FILES_CLASSICLADDER\n",
            "_FILES_CLASSICLADDER\nstray line\n_/FILES_CLASSICLADDER\n",
            "_FILES_CLASSICLADDER\n_FILE-a.txt\n_FILE-b.txt\n_/FILES_CLASSICLADDER\n",
        ];
        for case in cases {
            assert!(
                parse_container(case.as_bytes()).is_err(),
                "`{case}` should not parse"
            );
        }
    }

    #[test]
    fn empty_parameter_files_are_allowed() {
        let text =
            "_FILES_CLASSICLADDER\n_FILE-empty.txt\n_/FILE-empty.txt\n_/FILES_CLASSICLADDER\n";
        let parts = parse_container(text.as_bytes()).expect("parses");
        assert_eq!(parts, vec![("empty.txt".to_owned(), String::new())]);
        assert_eq!(serialize_container(&parts), text);
    }
}
