use anyhow::{Context, Result};
use clap::Parser;
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};

#[derive(Parser)]
#[command(name = "ttlint")]
#[command(about = "tiny text linter")]
struct Args {
    /// Additional patterns to search for (can be specified multiple times)
    #[arg(short = 'p', long = "pattern")]
    patterns: Vec<String>,

    /// Fix issues by removing matches
    #[arg(short = 'f', long = "fix")]
    fix: bool,

    /// Files to lint
    files: Vec<PathBuf>,
}

fn main() -> Result<()> {
    let args = Args::parse();
    let mut bad = false;
    for file_path in &args.files {
        bad |= lint_file(file_path, &args.patterns, args.fix)?;
    }
    if bad {
        std::process::exit(1);
    }
    Ok(())
}

fn lint_file(path: &Path, pats: &[String], fix: bool) -> Result<bool> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open file: {}", path.display()))?;
    let mut contents = Vec::new();
    file.read_to_end(&mut contents)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    let stderr = std::io::stderr();
    let mut lock = stderr.lock();
    let (bad, fixed) = lint_bytes(path, contents.as_slice(), pats, &mut lock, fix)?;

    if fixed.len() != contents.len() {
        assert!(fix);
        let mut file = fs::File::create(path)
            .with_context(|| format!("Failed to open file for writing: {}", path.display()))?;
        file.write_all(&fixed)
            .with_context(|| format!("Failed to write file: {}", path.display()))?;
    }
    Ok(bad)
}

pub(crate) fn lint_bytes<W: Write>(
    path: &Path,
    contents: &[u8],
    pats: &[String],
    writer: &mut W,
    fix: bool,
) -> std::result::Result<(bool, Vec<u8>), anyhow::Error> {
    let mut bad = contents.starts_with(&[0xEF, 0xBB, 0xBF]);
    if bad {
        writeln!(writer, "{}:1:1: UTF-8 byte-order mark", path.display())?;
    }
    let fixed = if bad && fix { &contents[3..] } else { contents };
    let (pat_bad, fixed) = lint_patterns(path, fixed, pats, writer, fix)?;
    bad |= pat_bad;
    Ok((bad, fixed))
}

struct Position {
    offset: usize,
    line: usize,
    col: usize,
}

pub(crate) fn lint_patterns<W: Write>(
    path: &Path,
    contents: &[u8],
    user_pats: &[String],
    writer: &mut W,
    fix: bool,
) -> Result<(bool, Vec<u8>), anyhow::Error> {
    let mut bad = false;
    let mut pats = vec!["\n<<<<<<<", "\n=======", "\n>>>>>>>", " \n", "\t\n", "\r"];
    let default_pat_count = pats.len();
    pats.extend(user_pats.iter().map(|s| s.as_str()));
    let ac =
        aho_corasick::AhoCorasick::new(&pats).context("Failed to build Aho-Corasick automaton")?;

    let mut fixed = Vec::with_capacity(contents.len());
    let mut last_end = 0;

    let mut cursor = Position {
        offset: 0,
        line: 1,
        col: 1,
    };

    for mat in ac.find_iter(contents) {
        let mut pos = mat.start();
        let end = mat.end();
        let pat_id = mat.pattern();
        let pat_idx = pat_id.as_usize();
        let pat = pats[pat_idx];
        if pat.starts_with('\n') {
            pos += 1;
        }

        bad = true;
        let contents_since_last_match = &contents[cursor.offset..pos];
        let lines_since_last_match = contents_since_last_match
            .iter()
            .filter(|&&b| b == b'\n')
            .count();
        let chars_since_last_line = contents_since_last_match
            .iter()
            .rev()
            .take_while(|&&b| b != b'\n')
            .count();

        let line = cursor.line + lines_since_last_match;
        let col = if lines_since_last_match == 0 {
            chars_since_last_line + cursor.col
        } else {
            chars_since_last_line + 1
        };

        cursor.offset = pos;
        cursor.line = line;
        cursor.col = col;

        let msg = match pat_idx {
            0 => "merge conflict start marker",
            1 => "merge conflict separator",
            2 => "merge conflict end marker",
            3 => "trailing whitespace",
            4 => "trailing whitespace",
            5 => "carriage return",
            _ => {
                let user_pattern_idx = pat_idx - default_pat_count;
                &user_pats[user_pattern_idx]
            }
        };
        writeln!(writer, "{}:{}:{}: {}", path.display(), line, col, msg)?;

        if fix {
            fixed.extend_from_slice(&contents[last_end..pos]);
            if pats[pat_idx].ends_with('\n') {
                fixed.push(b'\n');
            }
            last_end = end;
        }
    }

    if fix {
        fixed.extend_from_slice(&contents[last_end..]);
    } else {
        fixed = contents.to_vec();
    }

    Ok((bad, fixed))
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    #[test]
    fn ok() {
        let path = Path::new("test.txt");
        let contents = b"hello world";
        let pats = vec![];
        let mut output = Vec::new();

        let (bad, fixed) = lint_bytes(path, contents, &pats, &mut output, true).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello world"#]].assert_eq(&fixed_str);
        assert!(!bad);
    }

    #[test]
    fn bom() {
        let path = Path::new("test.txt");
        let contents = b"\xEF\xBB\xBFhello world";
        let pats = vec![];
        let mut output = Vec::new();

        let (bad, fixed) = lint_bytes(path, contents, &pats, &mut output, true).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello world"#]].assert_eq(&fixed_str);
        assert!(bad);
    }

    #[test]
    fn merge_conflict() {
        let path = Path::new("test.txt");
        let contents = b"some content\n>>>>>>> branch\n";
        let pats = vec![];
        let mut output = Vec::new();

        let (bad, fixed) = lint_bytes(path, contents, &pats, &mut output, true).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"some content
 branch
"#]]
        .assert_eq(&fixed_str);
        assert!(bad);
    }

    #[test]
    fn merge_conflict_not_at_line_start() {
        let path = Path::new("test.txt");
        let contents = b"some text <<<<<<< HEAD\nmore text ======= here\nand >>>>>>> branch\n";
        let pats = vec![];
        let mut output = Vec::new();

        let (bad, _fixed) = lint_bytes(path, contents, &pats, &mut output, false).unwrap();
        assert!(
            !bad,
            "Merge conflict markers in middle of line should not match"
        );
    }

    #[test]
    fn trailing_whitespace() {
        let path = Path::new("test.txt");
        let contents = b"line with trailing space \nline with trailing tab\t\nnext line\n";
        let pats = vec![];
        let mut output = Vec::new();

        let (bad, fixed) = lint_bytes(path, contents, &pats, &mut output, true).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"line with trailing space
line with trailing tab
next line
"#]]
        .assert_eq(&fixed_str);
        assert!(bad);
    }

    #[test]
    fn user_pat() {
        let path = Path::new("test.txt");
        let contents = b"hello FIXME world\nand TODO here\n";
        let pats = vec!["FIXME".to_string(), "TODO".to_string()];
        let mut output = Vec::new();

        let (bad, fixed) = lint_bytes(path, contents, &pats, &mut output, true).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello  world
and  here
"#]]
        .assert_eq(&fixed_str);
        assert!(bad);
    }
}
