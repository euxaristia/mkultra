# mkultra

A minimal, Unix-philosophy-compliant build tool in Pony.

## Usage

```
mkultra [target] [NAME=value ...] [-f FILE] [-j N] [-eikSnpqrst]
```

### Options

| Flag | Description |
|------|-------------|
| `-f FILE` | Read FILE as the makefile (default: Makefile, then makefile) |
| `-j N` | Run up to N recipes in parallel (default: 1) |
| `-e` | Environment variables override Makefile assignments |
| `-i` | Ignore errors from commands |
| `-k` | Keep going after errors |
| `-S` | Cancel a prior `-k` (errors stop the build) |
| `-n` | Dry run (print commands but don't execute) |
| `-p` | Print database (rules and variables) |
| `-q` | Question mode (exit 0 if up to date, 1 otherwise) |
| `-r` | Disable built-in rules (no-op, for compatibility) |
| `-s` | Silent mode (don't echo commands) |
| `-t` | Touch targets instead of running recipes |
| `-h` | Show help |
| `--version` | Show version |

Positional `NAME=value` arguments are macro overrides — they take precedence over Makefile assignments and over the environment.

Recipe lines may be prefixed with any combination of `@` (silent), `-` (ignore error), `+` (always run, even under `-n`/`-q`).

## Building

Requires [`ponyc`](https://www.ponylang.io/). No package manager, no third-party deps — Pony stdlib only.

```bash
make            # build ./mkultra
make install    # install to ~/.local/bin/mkultra
make uninstall  # remove it
```

## Features

- **Makefile parsing**: `target: prereq1 prereq2`, tab-indented recipes, `.PHONY`
- **Variable assignment**: `=`, `:=`, `?=`, `+=` with `$(VAR)` and `${VAR}` expansion (cycle-safe)
- **Substitution references**: `$(VAR:s1=s2)` replaces the `s1` suffix with `s2` in each word of `VAR`
- **DAG construction**: Dependency graph with topological sort
- **Circular dependency detection**
- **Mtime-based staleness**: Only rebuilds when prerequisites are newer
- **Automatic variables**: `$@`, `$<`, `$^` (dedup), `$+` (keeps dups), `$?` (newer prereqs)
- **Variable functions**: `$(wildcard pattern)`, `$(shell command)`
- **Process execution**: Runs recipes via `/bin/sh`
- **Parallel jobs**: `-j N` dispatches independent recipes concurrently (effective concurrency is bounded by `--ponythreads`, which defaults to the CPU count)
- **Error handling**: Exits on first failure, `-k` to continue

## Testing

```bash
make test

# Integration tests
cd tests/test1
../../mkultra
../../mkultra  # should show "up to date"
```

## License

This project is released into the public domain under the terms of the [UNLICENSE](UNLICENSE).
