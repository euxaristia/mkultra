# mkultra

A minimal, Unix-philosophy-compliant build tool in Rust.

## Usage

```
mkultra [target] [-f Makefile] [-j N] [-k] [-n]
```

### Options

| Flag | Description |
|------|-------------|
| `-f FILE` | Read FILE as the makefile (default: Makefile, then makefile) |
| `-j N` | Allow N parallel jobs |
| `-k` | Keep going after errors |
| `-n` | Dry run |
| `-h` | Show help |

## Building

```bash
cargo build --release
```

## Features

- **Makefile parsing**: `target: prereq1 prereq2`, tab-indented recipes, `.PHONY`
- **Variable assignment**: `VAR = value` with `$(VAR)` expansion
- **DAG construction**: Dependency graph with topological sort
- **Circular dependency detection**
- **Mtime-based staleness**: Only rebuilds when prerequisites are newer
- **Order-only prerequisites**: `target: normal | order_only` syntax
- **Automatic variables**:
  - `$@` - target name
  - `$<` - first prerequisite
  - `$^` - all prerequisites (unique)
  - `$+` - all prerequisites (with duplicates)
  - `$?` - prerequisites newer than target
  - `$*` - stem (for pattern rules)
  - `$|` - order-only prerequisites
  - `$(@D)`, `$(@F)` - directory/file parts of `$@`
  - `$(<D)`, `$(<F)` - directory/file parts of first prereq
- **Variable functions**:
  - `$(wildcard pattern)` - file globbing
  - `$(subst from,to,text)` - text substitution
  - `$(patsubst pattern,repl,text)` - pattern substitution
  - `$(shell command)` - shell command execution
  - `$(dir names...)` - directory parts
  - `$(notdir names...)` - filenames
  - `$(suffix names...)` - suffixes
  - `$(basename names...)` - basenames
  - `$(addsuffix sfx,names...)` - add suffix to each word
  - `$(addprefix pfx,names...)` - add prefix to each word
  - `$(filter pat...,text)` - filter words matching patterns
  - `$(filter-out pat...,text)` - filter out matching words
  - `$(sort list)` - sort and deduplicate
  - `$(word n,text)`, `$(words text)`, `$(firstword text)`, `$(lastword text)`
  - `$(strip text)`, `$(findstring find,in)`, `$(join list1,list2)`
- **Process execution**: Runs recipes via `/bin/sh`
- **Error handling**: Exits on first failure, `-k` to continue

## Testing

```bash
cargo test

# Integration tests
cd tests/test3 && ../../target/release/mkultra
cd tests/test3 && ../../target/release/mkultra  # should show "up to date"
```
