# ttlint

`ttlint` is a Tiny Text Linter. It uses the very fast [Aho-Corasick algorithm]
to search for a set of strings.

[Aho-Corasick algorithm]: https://en.wikipedia.org/wiki/Aho%E2%80%93Corasick_algorithm

By default, it searches for:

- Trailing whitespace
- The [UTF-8 byte-order mark](https://en.wikipedia.org/wiki/Byte_order_mark#UTF-8)
- Git merge conflict markers
- Carriage return (`\r`)

## Features

- Checks for the presence of certain substrings
- Very fast
- `--fix` mode available
- Precompiled binaries available
- Safe, doesn't panic
- <200 lines of code
<!-- - GitHub actions (TODO) -->

## Non-Features

- No recursive mode, use `find`/`xargs`/shell globs
- Not parallel, use `xargs`, `make`, or `ninja`

## Install

Download a binary from the [releases page][releases], or build with
[Cargo][cargo]:

```sh
cargo install --locked ttlint
```

[cargo]: https://doc.rust-lang.org/cargo/
[releases]: https://github.com/langston-barrett/ttlint/releases
