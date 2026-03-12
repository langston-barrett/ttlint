use anyhow::{Context, Result};
use clap::Parser;
use memchr::memchr_iter;
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

const DEFAULT_PATS: &[&str] = &["\n<<<<<<<", "\n=======", "\n>>>>>>>", " \n", "\t\n", "\r"];

const DEFAULT_MSGS: &[&str] = &[
    "merge conflict start marker",
    "merge conflict separator",
    "merge conflict end marker",
    "trailing whitespace",
    "trailing whitespace",
    "carriage return",
];

fn main() -> Result<()> {
    let args = Args::parse();
    let mut pats: Vec<&str> = DEFAULT_PATS.to_vec();
    pats.extend(args.patterns.iter().map(String::as_str));
    let ac =
        aho_corasick::AhoCorasick::new(&pats).context("Failed to build Aho-Corasick automaton")?;

    let stderr = std::io::stderr();
    let mut writer = std::io::BufWriter::new(stderr.lock());
    let mut contents = Vec::new();
    let mut fixed = Vec::new();
    let mut bad = false;
    for file_path in &args.files {
        let file_bad = lint_file(
            file_path,
            &ac,
            &pats,
            args.fix,
            &mut writer,
            &mut contents,
            &mut fixed,
        )?;
        if file_bad {
            writer.flush()?;
            bad = true;
        }
    }
    if bad {
        std::process::exit(1);
    }
    Ok(())
}

fn lint_file<W: Write>(
    path: &Path,
    ac: &aho_corasick::AhoCorasick,
    pats: &[&str],
    fix: bool,
    writer: &mut W,
    contents: &mut Vec<u8>,
    fixed: &mut Vec<u8>,
) -> Result<bool> {
    let mut file =
        fs::File::open(path).with_context(|| format!("Failed to open file: {}", path.display()))?;
    contents.clear();
    file.read_to_end(contents)
        .with_context(|| format!("Failed to read file: {}", path.display()))?;

    let bad = lint_bytes(path, contents.as_slice(), ac, pats, writer, fix, fixed)?;

    if fix && fixed.len() != contents.len() {
        let mut file = fs::File::create(path)
            .with_context(|| format!("Failed to open file for writing: {}", path.display()))?;
        file.write_all(fixed)
            .with_context(|| format!("Failed to write file: {}", path.display()))?;
    }
    Ok(bad)
}

pub(crate) fn lint_bytes<W: Write>(
    path: &Path,
    contents: &[u8],
    ac: &aho_corasick::AhoCorasick,
    pats: &[&str],
    writer: &mut W,
    fix: bool,
    fixed: &mut Vec<u8>,
) -> std::result::Result<bool, anyhow::Error> {
    let mut bad = contents.starts_with(&[0xEF, 0xBB, 0xBF]);
    if bad {
        writeln!(writer, "{}:1:1: UTF-8 byte-order mark", path.display())?;
    }
    let input = if bad && fix { &contents[3..] } else { contents };
    let pat_bad = lint_patterns(path, input, ac, pats, writer, fix, fixed)?;
    bad |= pat_bad;
    Ok(bad)
}

pub(crate) fn lint_patterns<W: Write>(
    path: &Path,
    contents: &[u8],
    ac: &aho_corasick::AhoCorasick,
    pats: &[&str],
    writer: &mut W,
    fix: bool,
    fixed: &mut Vec<u8>,
) -> Result<bool, anyhow::Error> {
    let mut bad = false;

    fixed.clear();
    if fix {
        fixed.reserve(contents.len().saturating_sub(fixed.capacity()));
    }
    let mut last_end = 0;

    let mut line = 1;
    let mut scanned = 0;
    let mut line_start = 0;

    for mat in ac.find_iter(contents) {
        let mut pos = mat.start();
        let end = mat.end();
        let pat_idx = mat.pattern().as_usize();
        let pat = pats[pat_idx];
        if pat.starts_with('\n') {
            pos += 1;
        }

        bad = true;
        for nl in memchr_iter(b'\n', &contents[scanned..pos]) {
            line += 1;
            line_start = scanned + nl + 1;
        }
        let col = pos - line_start + 1;
        scanned = pos;

        let msg = if pat_idx < DEFAULT_MSGS.len() {
            DEFAULT_MSGS[pat_idx]
        } else {
            pat
        };
        writeln!(writer, "{}:{}:{}: {msg}", path.display(), line, col)?;

        if fix {
            fixed.extend_from_slice(&contents[last_end..pos]);
            if pat.ends_with('\n') {
                fixed.push(b'\n');
            }
            last_end = end;
        }
    }

    if fix {
        fixed.extend_from_slice(&contents[last_end..]);
    }

    Ok(bad)
}

#[cfg(test)]
mod tests {
    use super::*;
    use expect_test::expect;

    fn test_ac() -> (aho_corasick::AhoCorasick, Vec<&'static str>) {
        let pats: Vec<&str> = DEFAULT_PATS.to_vec();
        let ac = aho_corasick::AhoCorasick::new(&pats).unwrap();
        (ac, pats)
    }

    fn test_ac_with<'a>(user_pats: &'a [&'a str]) -> (aho_corasick::AhoCorasick, Vec<&'a str>) {
        let mut pats: Vec<&str> = DEFAULT_PATS.to_vec();
        pats.extend_from_slice(user_pats);
        let ac = aho_corasick::AhoCorasick::new(&pats).unwrap();
        (ac, pats)
    }

    #[test]
    fn ok() {
        let path = Path::new("test.txt");
        let contents = b"hello world";
        let (ac, pats) = test_ac();
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, true, &mut fixed).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello world"#]].assert_eq(&fixed_str);
        assert!(!bad);
    }

    #[test]
    fn bom() {
        let path = Path::new("test.txt");
        let contents = b"\xEF\xBB\xBFhello world";
        let (ac, pats) = test_ac();
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, true, &mut fixed).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello world"#]].assert_eq(&fixed_str);
        assert!(bad);
    }

    #[test]
    fn merge_conflict() {
        let path = Path::new("test.txt");
        let contents = b"some content\n>>>>>>> branch\n";
        let (ac, pats) = test_ac();
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, true, &mut fixed).unwrap();
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
        let (ac, pats) = test_ac();
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, false, &mut fixed).unwrap();
        assert!(
            !bad,
            "Merge conflict markers in middle of line should not match"
        );
    }

    #[test]
    fn trailing_whitespace() {
        let path = Path::new("test.txt");
        let contents = b"line with trailing space \nline with trailing tab\t\nnext line\n";
        let (ac, pats) = test_ac();
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, true, &mut fixed).unwrap();
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
        let (ac, pats) = test_ac_with(&["FIXME", "TODO"]);
        let mut output = Vec::new();
        let mut fixed = Vec::new();

        let bad = lint_bytes(path, contents, &ac, &pats, &mut output, true, &mut fixed).unwrap();
        let fixed_str = String::from_utf8(fixed).unwrap();
        expect![[r#"hello  world
and  here
"#]]
        .assert_eq(&fixed_str);
        assert!(bad);
    }
}
