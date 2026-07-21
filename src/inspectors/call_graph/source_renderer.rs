//! Renders source locations for call graph output.

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};

/// Parse `src` as `offset:length` and return a human-readable line range like
/// `L5-L7`. Uses a cache of line-offset maps to avoid re-reading files.
pub fn offset_to_line_range(
    file_path: impl AsRef<Path>,
    src: &str,
    cache: &mut HashMap<PathBuf, Vec<usize>>,
) -> String {
    let file_path = file_path.as_ref();
    let (offset_str, length_str) = match src.split_once(':') {
        Some((o, l)) => (o, l),
        None => return src.to_string(),
    };

    let offset: usize = match offset_str.parse() {
        Ok(o) => o,
        Err(_) => return src.to_string(),
    };
    let length: usize = match length_str.parse() {
        Ok(l) => l,
        Err(_) => return src.to_string(),
    };

    let line_offsets = cache.entry(file_path.to_path_buf()).or_insert_with(|| {
        let content = match fs::read_to_string(file_path) {
            Ok(c) => c,
            Err(_) => return Vec::new(),
        };
        build_line_offsets(&content)
    });

    if line_offsets.is_empty() {
        return src.to_string();
    }

    let start_line = byte_offset_to_line(offset, line_offsets);
    let end_line = byte_offset_to_line(
        offset.saturating_add(length).saturating_sub(1),
        line_offsets,
    );

    if start_line == end_line {
        format!("L{}", start_line)
    } else {
        format!("L{}-L{}", start_line, end_line)
    }
}

/// Return the start line for a source range as a number without the `L` prefix.
pub fn offset_to_line(
    file_path: impl AsRef<Path>,
    src: &str,
    cache: &mut HashMap<PathBuf, Vec<usize>>,
) -> String {
    let range = offset_to_line_range(file_path, src, cache);
    range
        .strip_prefix('L')
        .and_then(|line| line.split('-').next())
        .unwrap_or(&range)
        .to_string()
}

/// Build a vector where `line_offsets[n]` is the byte offset of the start of
/// line `n` (1-indexed: `line_offsets[1]` is the offset of line 1).
fn build_line_offsets(content: &str) -> Vec<usize> {
    let mut offsets = vec![0, 0]; // offsets[0] is dummy, offsets[1] = start of line 1
    for (i, byte) in content.bytes().enumerate() {
        if byte == b'\n' {
            offsets.push(i + 1);
        }
    }
    offsets
}

/// Given a byte offset and a line-offsets vector, return the 1-indexed line number.
fn byte_offset_to_line(offset: usize, line_offsets: &[usize]) -> usize {
    match line_offsets.binary_search(&offset) {
        Ok(line) => line.max(1),
        Err(line) => line.saturating_sub(1).max(1),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn byte_offset_to_line_finds_correct_line() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        assert_eq!(byte_offset_to_line(0, &offsets), 1);
        assert_eq!(byte_offset_to_line(3, &offsets), 1);
        assert_eq!(byte_offset_to_line(6, &offsets), 2);
        assert_eq!(byte_offset_to_line(8, &offsets), 2);
        assert_eq!(byte_offset_to_line(12, &offsets), 3);
        assert_eq!(byte_offset_to_line(14, &offsets), 3);
    }

    #[test]
    fn offset_to_line_range_single_line() {
        let content = "line1\nline2\nline3\n";
        let offsets = build_line_offsets(content);
        let mut cache = HashMap::new();
        assert_eq!(
            offset_to_line_range("/tmp/file.sol", content, &mut cache),
            content
        );
        assert_eq!(byte_offset_to_line(0, &offsets), 1);
        assert_eq!(byte_offset_to_line(6, &offsets), 2);
        assert_eq!(byte_offset_to_line(12, &offsets), 3);
    }
}
