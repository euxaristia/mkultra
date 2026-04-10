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
- **Process execution**: Runs recipes via `/bin/sh`
- **Error handling**: Exits on first failure, `-k` to continue

## Testing

```bash
cargo test

# Integration tests
cd tests/test3 && ../../target/release/mkultra
cd tests/test3 && ../../target/release/mkultra  # should show "up to date"
```
