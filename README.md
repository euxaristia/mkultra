# mkultra

A minimal, Unix-philosophy-compliant build tool in Pony.

## Usage

```
mkultra [target] [-f Makefile] [-iknpqrs]
```

### Options

| Flag | Description |
|------|-------------|
| `-f FILE` | Read FILE as the makefile (default: Makefile, then makefile) |
| `-i` | Ignore errors from commands |
| `-k` | Keep going after errors |
| `-n` | Dry run (print commands but don't execute) |
| `-p` | Print database (rules and variables) |
| `-q` | Question mode (exit 0 if up to date, 1 otherwise) |
| `-r` | Disable built-in rules (no-op, for compatibility) |
| `-s` | Silent mode (don't echo commands) |
| `-h` | Show help |
| `--version` | Show version |

## Building

Requires [`ponyc`](https://www.ponylang.io/). No package manager, no third-party deps — Pony stdlib only.

```bash
make            # build ./mkultra
make install    # install to ~/.local/bin/mkultra
make uninstall  # remove it
```

## Features

- **Makefile parsing**: `target: prereq1 prereq2`, tab-indented recipes, `.PHONY`
- **Variable assignment**: `=`, `:=`, `?=`, `+=` with `$(VAR)` expansion (cycle-safe)
- **DAG construction**: Dependency graph with topological sort
- **Circular dependency detection**
- **Mtime-based staleness**: Only rebuilds when prerequisites are newer
- **Automatic variables**: `$@`, `$<`, `$^`
- **Variable functions**: `$(wildcard pattern)`, `$(shell command)`
- **Process execution**: Runs recipes via `/bin/sh`
- **Error handling**: Exits on first failure, `-k` to continue

## Testing

```bash
make test

# Integration tests
cd tests/test1 && ../../mkultra
cd tests/test1 && ../../mkultra  # should show "up to date"
```

## License

This project is released into the public domain under the terms of the [UNLICENSE](UNLICENSE).
