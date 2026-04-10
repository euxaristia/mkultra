//! mkultra - A minimal, Unix-philosophy-compliant build tool.
//!
//! Usage: mkultra [target] [-f Makefile] [-j N] [-k] [-n]

use std::collections::{HashMap, HashSet};
use std::env;
use std::fs;
use std::io::{self, Write};
use std::path::Path;
use std::process::{Command, ExitCode};
use std::time::SystemTime;

// ---------------------------------------------------------------------------
// CLI argument parsing
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
struct CliArgs {
    target: Option<String>,
    makefile: Option<String>,
    jobs: Option<usize>,
    keep_going: bool,
    dry_run: bool,
    help: bool,
}

fn parse_args() -> Result<CliArgs, String> {
    let mut args = CliArgs::default();
    let argv: Vec<String> = env::args().skip(1).collect();
    let mut i = 0;
    while i < argv.len() {
        match argv[i].as_str() {
            "-f" => {
                i += 1;
                if i >= argv.len() {
                    return Err("-f requires an argument".into());
                }
                args.makefile = Some(argv[i].clone());
            }
            "-j" => {
                i += 1;
                if i >= argv.len() {
                    return Err("-j requires an argument".into());
                }
                args.jobs = Some(argv[i].parse().map_err(|_| "invalid -j value")?);
            }
            "-k" => args.keep_going = true,
            "-n" => args.dry_run = true,
            "-h" | "--help" => args.help = true,
            s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
            s => args.target = Some(s.to_string()),
        }
        i += 1;
    }
    Ok(args)
}

fn print_usage() {
    println!("Usage: mkultra [target] [-f Makefile] [-j N] [-k] [-n]");
    println!();
    println!("Options:");
    println!("  -f FILE   Read FILE as the makefile (default: Makefile, then makefile)");
    println!("  -j N      Allow N parallel jobs");
    println!("  -k        Keep going after errors");
    println!("  -n        Dry run");
    println!("  -h        Show this help");
}

// ---------------------------------------------------------------------------
// DAG node
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct DagNode {
    target: String,
    prereqs: Vec<String>,
    order_prereqs: Vec<String>,
    recipes: Vec<String>,
    is_phony: bool,
    stem: Option<String>,
}

impl DagNode {
    fn new(target: String, is_phony: bool) -> Self {
        Self {
            target,
            prereqs: Vec::new(),
            order_prereqs: Vec::new(),
            recipes: Vec::new(),
            is_phony,
            stem: None,
        }
    }

    /// Returns `true` if this target needs rebuilding based on mtime comparison.
    fn needs_rebuild(&self) -> bool {
        if self.is_phony {
            return true;
        }
        let target_mtime = file_mtime(&self.target);
        if target_mtime == 0 {
            return true;
        }
        for prereq in &self.prereqs {
            let prereq_mtime = file_mtime(prereq);
            if prereq_mtime == 0 || prereq_mtime > target_mtime {
                return true;
            }
        }
        false
    }

    /// Returns prerequisites newer than the target (for $?).
    fn newer_prereqs(&self) -> Vec<&str> {
        let target_mtime = file_mtime(&self.target);
        self.prereqs
            .iter()
            .filter(|p| {
                let mtime = file_mtime(p);
                mtime > target_mtime
            })
            .map(|s| s.as_str())
            .collect()
    }

    /// Returns the directory part of a path, or "." if none.
    fn dir_part(path: &str) -> &str {
        path.rsplit_once('/').map(|(d, _)| d).unwrap_or(".")
    }

    /// Returns the file part of a path (everything after last /).
    fn file_part(path: &str) -> &str {
        path.rsplit_once('/').map(|(_, f)| f).unwrap_or(path)
    }
}

/// Get the mtime of a file as seconds since Unix epoch. Returns 0 if missing.
fn file_mtime(path: &str) -> u64 {
    fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(SystemTime::UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

// ---------------------------------------------------------------------------
// DAG
// ---------------------------------------------------------------------------

#[derive(Debug)]
struct Dag {
    nodes: HashMap<String, DagNode>,
    variables: HashMap<String, String>,
    default_target: Option<String>,
}

impl Dag {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            variables: HashMap::new(),
            default_target: None,
        }
    }

    fn set_variable(&mut self, name: &str, value: String) {
        self.variables.insert(name.to_string(), value);
    }

    fn ensure_node(&mut self, target: &str, is_phony: bool) -> &mut DagNode {
        self.nodes
            .entry(target.to_string())
            .or_insert_with(|| DagNode::new(target.to_string(), is_phony))
    }

    fn add_prereq(&mut self, target: &str, prereq: &str) {
        self.ensure_node(target, false)
            .prereqs
            .push(prereq.to_string());
        self.ensure_node(prereq, false);
        if self.default_target.is_none()
            && !target.starts_with('.')
            && target != ".PHONY"
            && target != ".SUFFIXES"
        {
            self.set_default(target);
        }
    }

    fn add_order_prereq(&mut self, target: &str, prereq: &str) {
        self.ensure_node(target, false)
            .order_prereqs
            .push(prereq.to_string());
        self.ensure_node(prereq, false);
        if self.default_target.is_none()
            && !target.starts_with('.')
            && target != ".PHONY"
            && target != ".SUFFIXES"
        {
            self.set_default(target);
        }
    }

    fn set_stem(&mut self, target: &str, stem: Option<String>) {
        if let Some(node) = self.nodes.get_mut(target) {
            node.stem = stem;
        }
    }

    fn add_recipe(&mut self, target: &str, recipe: &str) {
        self.ensure_node(target, false)
            .recipes
            .push(recipe.to_string());
    }

    fn set_default(&mut self, target: &str) {
        if self.default_target.is_none() {
            self.default_target = Some(target.to_string());
        }
    }

    /// Detect circular dependencies. Returns the cycle path if found.
    fn detect_cycle(&self) -> Option<Vec<String>> {
        // 0 = white (unvisited), 1 = gray (in progress), 2 = black (done)
        let mut color: HashMap<String, u8> = self.nodes.keys().map(|k| (k.clone(), 0)).collect();
        let mut parent: HashMap<String, String> = HashMap::new();

        let all_keys: Vec<String> = self.nodes.keys().cloned().collect();
        for start in &all_keys {
            if color.get(start).copied().unwrap_or(2) != 0 {
                continue;
            }
            // iterative DFS using explicit state: (node, prereq_index)
            let mut stack: Vec<(String, usize)> = vec![(start.clone(), 0)];
            color.entry(start.clone()).or_insert(0);

            while let Some((node, idx)) = stack.last().cloned() {
                let c = color[&node];
                if c == 2 {
                    stack.pop();
                    continue;
                }

                // Mark gray on first visit
                if c == 0 {
                    color.insert(node.clone(), 1);
                }

                // Get prereqs and continue from where we left off
                let prereqs = self
                    .nodes
                    .get(&node)
                    .map(|nd| nd.prereqs.clone())
                    .unwrap_or_default();

                let mut pushed = false;
                let mut next_idx = idx;
                while next_idx < prereqs.len() {
                    let prereq = &prereqs[next_idx];
                    let pc = color.get(prereq).copied().unwrap_or(0);
                    if pc == 0 {
                        parent.insert(prereq.clone(), node.clone());
                        color.entry(prereq.clone()).or_insert(0);
                        // Update current node's index and push prereq
                        if let Some(top) = stack.last_mut() {
                            top.1 = next_idx + 1;
                        }
                        stack.push((prereq.clone(), 0));
                        pushed = true;
                        break;
                    } else if pc == 1 {
                        // Back edge = cycle
                        let mut cycle = vec![prereq.clone(), node.clone()];
                        let mut cur = node.clone();
                        for _ in 0..100 {
                            if let Some(p) = parent.get(&cur) {
                                if p == prereq {
                                    break;
                                }
                                cycle.push(p.clone());
                                cur = p.clone();
                            } else {
                                break;
                            }
                        }
                        cycle.reverse();
                        return Some(cycle);
                    }
                    // pc == 2: already done, skip
                    next_idx += 1;
                }

                if !pushed {
                    // All prereqs processed, mark black
                    color.insert(node.clone(), 2);
                    stack.pop();
                }
            }
        }
        None
    }

    /// Topological sort from a given target, returning nodes in build order.
    fn order(&self, target: &str) -> Result<Vec<&DagNode>, String> {
        let mut result = Vec::new();
        let mut visited = HashSet::new();
        self._topo(target, &mut visited, &mut result)?;
        let nodes: Vec<&DagNode> = result
            .iter()
            .filter_map(|name| self.nodes.get(name))
            .collect();
        Ok(nodes)
    }

    fn _topo(
        &self,
        name: &str,
        visited: &mut HashSet<String>,
        out: &mut Vec<String>,
    ) -> Result<(), String> {
        if !visited.insert(name.to_string()) {
            return Ok(());
        }
        if let Some(nd) = self.nodes.get(name) {
            for prereq in &nd.prereqs {
                self._topo(prereq, visited, out)?;
            }
        }
        out.push(name.to_string());
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Makefile parser
// ---------------------------------------------------------------------------

fn parse_makefile(content: &str, dag: &mut Dag) -> Result<(), String> {
    let mut cur_tgt = String::new();
    let mut phony_targets: Vec<String> = Vec::new();

    for raw_line in content.lines() {
        // Recipe lines start with a tab
        if raw_line.starts_with('\t') {
            let trimmed = raw_line.trim();
            if !trimmed.is_empty() && !cur_tgt.is_empty() {
                dag.add_recipe(&cur_tgt, trimmed);
            }
            continue;
        }

        let trimmed = raw_line.trim();
        if trimmed.is_empty() {
            cur_tgt.clear();
            continue;
        }
        if trimmed.starts_with('#') {
            continue;
        }

        // Check for variable assignment (VAR = value)
        if let Some(eq_pos) = trimmed.find('=') {
            let lhs = trimmed[..eq_pos].trim();
            let rhs = trimmed[eq_pos + 1..].trim();
            if !lhs.is_empty() && !lhs.contains(' ') && !lhs.contains('=') {
                dag.set_variable(lhs, rhs.to_string());
                cur_tgt.clear();
                continue;
            }
        }

        // Look for `target: prereqs` (rule line, not a variable assignment)
        if let Some(colon_pos) = trimmed.find(':') {
            // Make sure it's not a variable assignment `:=` or `::=` etc.
            let after_colon = &trimmed[colon_pos + 1..];
            if after_colon.starts_with('=') {
                cur_tgt.clear();
                continue;
            }

            let target_part = trimmed[..colon_pos].trim();
            cur_tgt = target_part.to_string();
            let is_phony = target_part == ".PHONY";
            dag.ensure_node(target_part, is_phony);

            // Handle .PHONY: the listed targets are phony
            if target_part == ".PHONY" {
                let prereqs_part = after_colon.trim();
                for name in prereqs_part.split_whitespace() {
                    phony_targets.push(name.to_string());
                    dag.ensure_node(name, true);
                }
            } else if target_part != ".SUFFIXES"
                && !target_part.starts_with('.')
                && dag.default_target.is_none()
            {
                dag.set_default(target_part);
            }

            let prereqs_part = after_colon.trim();
            if !prereqs_part.is_empty() && target_part != ".PHONY" {
                let prereqs: Vec<&str> = prereqs_part.split_whitespace().collect();
                let mut seen_bar = false;
                for prereq in prereqs {
                    if prereq == "|" {
                        seen_bar = true;
                        continue;
                    }
                    if seen_bar {
                        dag.add_order_prereq(target_part, prereq);
                    } else {
                        dag.add_prereq(target_part, prereq);
                    }
                }
            }
        }
    }

    // Mark phony targets
    for name in &phony_targets {
        if let Some(node) = dag.nodes.get_mut(name) {
            node.is_phony = true;
        }
    }

    Ok(())
}

// ---------------------------------------------------------------------------
// Automatic variable expansion
// ---------------------------------------------------------------------------

enum AutoVar {
    Target,
    FirstPrereq,
    AllPrereqs,
    AllPrereqsDups,
    NewerPrereqs,
    Stem,
    OrderOnly,
}

impl AutoVar {
    fn from_char(c: char) -> Option<Self> {
        match c {
            '@' => Some(Self::Target),
            '<' => Some(Self::FirstPrereq),
            '^' => Some(Self::AllPrereqs),
            '+' => Some(Self::AllPrereqsDups),
            '?' => Some(Self::NewerPrereqs),
            '*' => Some(Self::Stem),
            '|' => Some(Self::OrderOnly),
            _ => None,
        }
    }

    fn expand(&self, node: &DagNode) -> String {
        match self {
            Self::Target => node.target.clone(),
            Self::FirstPrereq => node.prereqs.first().cloned().unwrap_or_default(),
            Self::AllPrereqs => node.prereqs.join(" "),
            Self::AllPrereqsDups => node.prereqs.join(" "),
            Self::NewerPrereqs => node
                .newer_prereqs()
                .into_iter()
                .collect::<Vec<_>>()
                .join(" "),
            Self::Stem => node.stem.clone().unwrap_or_default(),
            Self::OrderOnly => node.order_prereqs.join(" "),
        }
    }

    fn dir_part(&self, node: &DagNode) -> String {
        let path = self.expand(node);
        if path.is_empty() {
            String::new()
        } else {
            DagNode::dir_part(&path).to_string()
        }
    }

    fn file_part(&self, node: &DagNode) -> String {
        let path = self.expand(node);
        if path.is_empty() {
            String::new()
        } else {
            DagNode::file_part(&path).to_string()
        }
    }
}

fn expand_vars(cmd: &str, node: &DagNode, variables: &HashMap<String, String>) -> String {
    let mut out = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 >= chars.len() {
                out.push('$');
                i += 1;
                continue;
            }

            let next = chars[i + 1];

            if next == '$' {
                out.push('$');
                i += 2;
                continue;
            }

            if next == '(' {
                if let Some((name, args, consumed)) = parse_function_call(&chars, i + 1) {
                    let expanded = expand_function(&name, &args, node, variables);
                    out.push_str(&expanded);
                    i += consumed;
                    continue;
                }
            }

            if let Some(var) = AutoVar::from_char(next) {
                out.push_str(&var.expand(node));
                i += 2;
                continue;
            }

            if next.is_alphanumeric() || next == '_' {
                if let Some((name, consumed)) = parse_simple_var(&chars, i + 1) {
                    let expanded = variables.get(&name).cloned().unwrap_or_default();
                    out.push_str(&expanded);
                    i += consumed;
                    continue;
                }
            }

            out.push('$');
            i += 1;
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

fn parse_simple_var(chars: &[char], start: usize) -> Option<(String, usize)> {
    let mut end = start;
    while end < chars.len() && (chars[end].is_alphanumeric() || chars[end] == '_') {
        end += 1;
    }
    if end > start {
        let name: String = chars[start..end].iter().collect();
        Some((name, 1 + end - start))
    } else {
        None
    }
}

fn parse_function_call(chars: &[char], start: usize) -> Option<(String, String, usize)> {
    let mut depth = 1;
    let mut i = start + 1;

    while i < chars.len() && depth > 0 {
        match chars[i] {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ => {}
        }
        i += 1;
    }

    if depth == 0 {
        let full: String = chars[start + 1..i - 1].iter().collect();
        if let Some((name, args)) = full.split_once('(') {
            let args = &args[..args.len().saturating_sub(1)];
            return Some((name.to_string(), args.to_string(), i - start + 1));
        }
        if let Some((name, args)) = full.split_once(',') {
            return Some((name.to_string(), args.to_string(), i - start + 1));
        }
        Some((full, String::new(), i - start + 1))
    } else {
        None
    }
}

// ---------------------------------------------------------------------------
// Variable functions
// ---------------------------------------------------------------------------

fn expand_function(
    name: &str,
    args: &str,
    node: &DagNode,
    variables: &HashMap<String, String>,
) -> String {
    match name {
        "@D" => AutoVar::Target.dir_part(node),
        "@F" => AutoVar::Target.file_part(node),
        "<D" => AutoVar::FirstPrereq.dir_part(node),
        "<F" => AutoVar::FirstPrereq.file_part(node),
        "^D" => AutoVar::AllPrereqs.dir_part(node),
        "^F" => AutoVar::AllPrereqs.file_part(node),
        "+D" => AutoVar::AllPrereqsDups.dir_part(node),
        "+F" => AutoVar::AllPrereqsDups.file_part(node),
        "?D" => AutoVar::NewerPrereqs.dir_part(node),
        "?F" => AutoVar::NewerPrereqs.file_part(node),
        "*D" => AutoVar::Stem.dir_part(node),
        "*F" => AutoVar::Stem.file_part(node),
        "|D" => AutoVar::OrderOnly.dir_part(node),
        "|F" => AutoVar::OrderOnly.file_part(node),
        "wildcard" => expand_wildcard(args),
        "subst" => expand_subst(args),
        "patsubst" => expand_patsubst(args),
        "shell" => expand_shell(args),
        "dir" => expand_dir(args),
        "notdir" => expand_notdir(args),
        "suffix" => expand_suffix(args),
        "basename" => expand_basename(args),
        "addsuffix" => expand_addsuffix(args),
        "addprefix" => expand_addprefix(args),
        "filter" => expand_filter(args),
        "filter-out" => expand_filter_out(args),
        "sort" => expand_sort(args),
        "word" => expand_word(args),
        "words" => expand_words(args),
        "firstword" => expand_firstword(args),
        "lastword" => expand_lastword(args),
        "strip" => expand_strip(args),
        "findstring" => expand_findstring(args),
        "join" => expand_join(args),
        _ => variables.get(name).cloned().unwrap_or_default(),
    }
}

fn split_args(s: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut current = String::new();
    let mut depth = 0;
    let mut in_paren = false;

    for ch in s.chars() {
        match ch {
            '(' => {
                depth += 1;
                current.push(ch);
            }
            ')' => {
                depth -= 1;
                current.push(ch);
            }
            ',' if depth == 0 && !in_paren => {
                result.push(current.trim().to_string());
                current.clear();
            }
            _ => {
                if ch == '\'' || ch == '"' {
                    in_paren = !in_paren;
                }
                current.push(ch);
            }
        }
    }
    if !current.trim().is_empty() {
        result.push(current.trim().to_string());
    }
    result
}

fn expand_wildcard(pattern: &str) -> String {
    let pattern = pattern.trim();
    if !pattern.contains('*') && !pattern.contains('?') {
        return pattern.to_string();
    }

    match glob::Pattern::new(pattern) {
        Ok(pat) => {
            let dir = if let Some((d, _)) = pattern.rsplit_once('/') {
                d.to_string()
            } else {
                ".".to_string()
            };
            let matches: Vec<String> = walkdir::WalkDir::new(&dir)
                .max_depth(1)
                .into_iter()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_type().is_file())
                .filter_map(|e| {
                    e.path().to_str().and_then(|p| {
                        if pat.matches(p) {
                            Some(p.to_string())
                        } else {
                            None
                        }
                    })
                })
                .collect();
            matches.join(" ")
        }
        Err(_) => String::new(),
    }
}

fn expand_subst(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 3 {
        return args.to_string();
    }
    let from = &parts[0];
    let to = &parts[1];
    let text = &parts[2];
    text.replace(from, to)
}

fn expand_patsubst(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 3 {
        return args.to_string();
    }
    let pattern = parts[0].trim();
    let replacement = parts[1].trim();
    let text = parts[2].trim();

    let mut result = Vec::new();
    for word in text.split_whitespace() {
        if pattern.contains('%') {
            let pct_idx = pattern.find('%').unwrap();
            let pfx = &pattern[..pct_idx];
            let sfx = &pattern[pct_idx + 1..];
            if word.starts_with(pfx) && (sfx.is_empty() || word.ends_with(sfx)) {
                let rest = &word[pfx.len()..word.len().saturating_sub(sfx.len())];
                let repl = replacement.replace("%", rest);
                result.push(repl);
            } else {
                result.push(word.to_string());
            }
        } else if word == pattern {
            result.push(replacement.to_string());
        } else {
            result.push(word.to_string());
        }
    }
    result.join(" ")
}

fn expand_shell(cmd: &str) -> String {
    let cmd = cmd.trim();
    let output = Command::new("/bin/sh").arg("-c").arg(cmd).output();
    match output {
        Ok(out) => {
            let stdout = String::from_utf8_lossy(&out.stdout);
            stdout.split_whitespace().collect::<Vec<_>>().join(" ")
        }
        Err(_) => String::new(),
    }
}

fn expand_dir(args: &str) -> String {
    args.split_whitespace()
        .map(|p| {
            if let Some((d, _)) = p.rsplit_once('/') {
                if d.is_empty() {
                    "."
                } else {
                    d
                }
            } else {
                "."
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_notdir(args: &str) -> String {
    args.split_whitespace()
        .map(|p| {
            if let Some((_, f)) = p.rsplit_once('/') {
                f
            } else {
                p
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_suffix(args: &str) -> String {
    args.split_whitespace()
        .filter_map(|p| {
            if let Some(dot) = p.rfind('.') {
                if dot > p.rfind('/').unwrap_or(0) {
                    return Some(&p[dot..]);
                }
            }
            None
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_basename(args: &str) -> String {
    args.split_whitespace()
        .map(|p| {
            if let Some(dot) = p.rfind('.') {
                if dot > p.rfind('/').unwrap_or(0) {
                    return &p[..dot];
                }
            }
            p
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_addsuffix(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 2 {
        return args.to_string();
    }
    let suffix = parts[0].trim();
    let words: Vec<&str> = parts[1].split_whitespace().collect();
    words
        .iter()
        .map(|w| format!("{}{}", w, suffix))
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_addprefix(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 2 {
        return args.to_string();
    }
    let prefix = parts[0].trim();
    let words: Vec<&str> = parts[1].split_whitespace().collect();
    words
        .iter()
        .map(|w| format!("{}{}", prefix, w))
        .collect::<Vec<_>>()
        .join(" ")
}

fn expand_filter(args: &str) -> String {
    if let Some((patterns_str, text)) = args.split_once(',') {
        let patterns: Vec<&str> = patterns_str.split_whitespace().collect();
        let words: Vec<&str> = text.split_whitespace().collect();

        let mut result = Vec::new();
        for word in words {
            for pattern in &patterns {
                if match glob::Pattern::new(pattern) {
                    Ok(pat) => pat.matches(word),
                    Err(_) => {
                        if pattern.contains('%') {
                            let pct_idx = pattern.find('%').unwrap();
                            let prefix = &pattern[..pct_idx];
                            let suffix = &pattern[pct_idx + 1..];
                            word.starts_with(prefix)
                                && (suffix.is_empty() || word.ends_with(suffix))
                        } else {
                            &*word == *pattern
                        }
                    }
                } {
                    result.push(word);
                    break;
                }
            }
        }
        result.join(" ")
    } else {
        String::new()
    }
}

fn expand_filter_out(args: &str) -> String {
    if let Some((patterns_str, text)) = args.split_once(',') {
        let patterns: Vec<&str> = patterns_str.split_whitespace().collect();
        let words: Vec<&str> = text.split_whitespace().collect();

        let mut result = Vec::new();
        for word in words {
            let mut matched = false;
            for pattern in &patterns {
                matched = match glob::Pattern::new(pattern) {
                    Ok(pat) => pat.matches(word),
                    Err(_) => {
                        if pattern.contains('%') {
                            let pct_idx = pattern.find('%').unwrap();
                            let prefix = &pattern[..pct_idx];
                            let suffix = &pattern[pct_idx + 1..];
                            word.starts_with(prefix)
                                && (suffix.is_empty() || word.ends_with(suffix))
                        } else {
                            &*word == *pattern
                        }
                    }
                };
                if matched {
                    break;
                }
            }
            if !matched {
                result.push(word);
            }
        }
        result.join(" ")
    } else {
        args.to_string()
    }
}

fn expand_sort(args: &str) -> String {
    let mut words: Vec<String> = args.split_whitespace().map(|s| s.to_string()).collect();
    words.sort();
    words.dedup();
    words.join(" ")
}

fn expand_word(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 2 {
        return String::new();
    }
    let n: usize = parts[0].trim().parse().unwrap_or(0);
    let words: Vec<&str> = parts[1].split_whitespace().collect();
    if n > 0 && n <= words.len() {
        words[n - 1].to_string()
    } else {
        String::new()
    }
}

fn expand_words(args: &str) -> String {
    args.split_whitespace().count().to_string()
}

fn expand_firstword(args: &str) -> String {
    args.split_whitespace().next().unwrap_or("").to_string()
}

fn expand_lastword(args: &str) -> String {
    args.split_whitespace().last().unwrap_or("").to_string()
}

fn expand_strip(args: &str) -> String {
    args.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn expand_findstring(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 2 {
        return String::new();
    }
    let find = parts[0].trim();
    let in_text = parts[1].trim();
    if in_text.contains(find) {
        find.to_string()
    } else {
        String::new()
    }
}

fn expand_join(args: &str) -> String {
    let parts = split_args(args);
    if parts.len() < 2 {
        return String::new();
    }
    let list1: Vec<&str> = parts[0].split_whitespace().collect();
    let list2: Vec<&str> = parts[1].split_whitespace().collect();
    let max_len = list1.len().max(list2.len());
    let mut result = Vec::new();
    for i in 0..max_len {
        let a = list1.get(i).unwrap_or(&"");
        let b = list2.get(i).unwrap_or(&"");
        result.push(format!("{}{}", a, b));
    }
    result.join(" ")
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

struct Executor {
    keep_going: bool,
    dry_run: bool,
    errors: usize,
    variables: HashMap<String, String>,
}

impl Executor {
    fn new(keep_going: bool, dry_run: bool, variables: HashMap<String, String>) -> Self {
        Self {
            keep_going,
            dry_run,
            errors: 0,
            variables,
        }
    }

    fn run(&mut self, nodes: &[&DagNode]) -> bool {
        for nd in nodes {
            if !nd.needs_rebuild() {
                println!("mkultra: '{}' is up to date.", nd.target);
                continue;
            }
            if nd.recipes.is_empty() {
                continue;
            }
            if !self.exec_node(nd) && !self.keep_going {
                return false;
            }
        }
        self.errors == 0
    }

    fn exec_node(&mut self, nd: &DagNode) -> bool {
        for recipe in &nd.recipes {
            let echo = !recipe.starts_with('@');
            let cmd = recipe.strip_prefix('@').unwrap_or(recipe);
            let expanded = expand_vars(cmd, nd, &self.variables);

            if self.dry_run {
                if echo {
                    println!("{expanded}");
                }
                continue;
            }

            if echo {
                println!("{expanded}");
                io::stdout().flush().ok();
            }

            let code = run_cmd(&expanded);
            if code != 0 {
                self.errors += 1;
                eprintln!("mkultra: *** [{}] Error {code}", nd.target);
                return false;
            }
        }
        true
    }
}

fn run_cmd(cmd: &str) -> i32 {
    let output = Command::new("/bin/sh").arg("-c").arg(cmd).status();
    match output {
        Ok(status) => status.code().unwrap_or(128),
        Err(_) => 127,
    }
}

// ---------------------------------------------------------------------------
// Main
// ---------------------------------------------------------------------------

fn main() -> ExitCode {
    let args = match parse_args() {
        Ok(a) => a,
        Err(e) => {
            eprintln!("mkultra: {e}");
            return ExitCode::from(2);
        }
    };

    if args.help {
        print_usage();
        return ExitCode::SUCCESS;
    }

    // Resolve makefile
    let makefile = match &args.makefile {
        Some(f) => f.clone(),
        None => {
            if Path::new("Makefile").exists() {
                "Makefile".to_string()
            } else if Path::new("makefile").exists() {
                "makefile".to_string()
            } else {
                eprintln!("mkultra: *** No makefile found.");
                return ExitCode::from(2);
            }
        }
    };

    // Read makefile
    let content = match fs::read_to_string(&makefile) {
        Ok(c) => c,
        Err(_) => {
            eprintln!("mkultra: *** Cannot open {makefile}");
            return ExitCode::from(2);
        }
    };

    // Parse
    let mut dag = Dag::new();
    if let Err(e) = parse_makefile(&content, &mut dag) {
        eprintln!("mkultra: {e}");
        return ExitCode::from(2);
    }

    // Cycle detection
    if let Some(cycle) = dag.detect_cycle() {
        eprintln!("mkultra: *** Circular dependency: {}", cycle.join(" -> "));
        return ExitCode::from(2);
    }

    // Determine target
    let build_target = match &args.target {
        Some(t) => {
            if !dag.nodes.contains_key(t) {
                eprintln!("mkultra: *** No rule to make {t}");
                return ExitCode::from(2);
            }
            t.clone()
        }
        None => match &dag.default_target {
            Some(dt) => dt.clone(),
            None => {
                eprintln!("mkultra: *** No default target.");
                return ExitCode::from(2);
            }
        },
    };

    // Topological order
    let nodes = match dag.order(&build_target) {
        Ok(n) => n,
        Err(e) => {
            eprintln!("mkultra: *** {e}");
            return ExitCode::from(2);
        }
    };

    // Execute
    let mut executor = Executor::new(args.keep_going, args.dry_run, dag.variables.clone());
    if executor.run(&nodes) {
        ExitCode::SUCCESS
    } else {
        ExitCode::from(1)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- parse_args --

    #[test]
    fn test_parse_args_empty() {
        // We can't easily test the real argv, so test the logic indirectly
        let args = parse_args_vec(&[] as &[&str]);
        assert!(args.is_ok());
        let args = args.unwrap();
        assert!(args.target.is_none());
        assert!(args.makefile.is_none());
        assert!(!args.keep_going);
        assert!(!args.dry_run);
    }

    #[test]
    fn test_parse_args_target() {
        let args = parse_args_vec(&["all"]).unwrap();
        assert_eq!(args.target.as_deref(), Some("all"));
    }

    #[test]
    fn test_parse_args_f() {
        let args = parse_args_vec(&["-f", "CustomMakefile"]).unwrap();
        assert_eq!(args.makefile.as_deref(), Some("CustomMakefile"));
    }

    #[test]
    fn test_parse_args_k() {
        let args = parse_args_vec(&["-k"]).unwrap();
        assert!(args.keep_going);
    }

    #[test]
    fn test_parse_args_n() {
        let args = parse_args_vec(&["-n"]).unwrap();
        assert!(args.dry_run);
    }

    #[test]
    fn test_parse_args_j() {
        let args = parse_args_vec(&["-j", "4"]).unwrap();
        assert_eq!(args.jobs, Some(4));
    }

    #[test]
    fn test_parse_args_combined() {
        let args =
            parse_args_vec(&["build", "-f", "Makefile.test", "-j", "2", "-k", "-n"]).unwrap();
        assert_eq!(args.target.as_deref(), Some("build"));
        assert_eq!(args.makefile.as_deref(), Some("Makefile.test"));
        assert_eq!(args.jobs, Some(2));
        assert!(args.keep_going);
        assert!(args.dry_run);
    }

    #[test]
    fn test_parse_args_unknown_option() {
        let err = parse_args_vec(&["--unknown"]).unwrap_err();
        assert!(err.contains("unknown option"));
    }

    #[test]
    fn test_parse_args_help() {
        let args = parse_args_vec(&["-h"]).unwrap();
        assert!(args.help);
    }

    /// Helper that parses a slice of &str as if they were CLI args
    fn parse_args_vec(args: &[&str]) -> Result<CliArgs, String> {
        let mut result = CliArgs::default();
        let mut i = 0;
        while i < args.len() {
            match args[i] {
                "-f" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("-f requires an argument".into());
                    }
                    result.makefile = Some(args[i].to_string());
                }
                "-j" => {
                    i += 1;
                    if i >= args.len() {
                        return Err("-j requires an argument".into());
                    }
                    result.jobs = Some(args[i].parse().map_err(|_| "invalid -j value")?);
                }
                "-k" => result.keep_going = true,
                "-n" => result.dry_run = true,
                "-h" | "--help" => result.help = true,
                s if s.starts_with('-') => return Err(format!("unknown option: {s}")),
                s => result.target = Some(s.to_string()),
            }
            i += 1;
        }
        Ok(result)
    }

    // -- DAG construction & cycle detection --

    #[test]
    fn test_dag_simple() {
        let mut dag = Dag::new();
        dag.add_prereq("all", "output.txt");
        dag.add_recipe("all", "echo done");
        dag.add_prereq("output.txt", "input.txt");
        dag.add_recipe("output.txt", "cat input.txt > output.txt");

        assert!(dag.nodes.contains_key("all"));
        assert!(dag.nodes.contains_key("output.txt"));
        assert!(dag.nodes.contains_key("input.txt"));
        assert_eq!(dag.nodes["all"].prereqs, vec!["output.txt"]);
    }

    #[test]
    fn test_dag_no_cycle() {
        let mut dag = Dag::new();
        dag.add_prereq("a", "b");
        dag.add_prereq("b", "c");
        assert!(dag.detect_cycle().is_none());
    }

    #[test]
    fn test_dag_cycle_detected() {
        let mut dag = Dag::new();
        dag.add_prereq("a", "b");
        dag.add_prereq("b", "c");
        dag.add_prereq("c", "a");
        let cycle = dag.detect_cycle();
        assert!(cycle.is_some());
        let cycle = cycle.unwrap();
        assert!(cycle.contains(&"a".to_string()));
        assert!(cycle.contains(&"b".to_string()));
        assert!(cycle.contains(&"c".to_string()));
    }

    #[test]
    fn test_dag_default_target() {
        let mut dag = Dag::new();
        dag.add_prereq("all", "output.txt");
        dag.add_prereq("output.txt", "input.txt");
        assert_eq!(dag.default_target.as_deref(), Some("all"));
    }

    #[test]
    fn test_dag_default_skips_phony_dotfiles() {
        let mut dag = Dag::new();
        dag.ensure_node(".PHONY", true);
        dag.ensure_node(".SUFFIXES", false);
        dag.add_prereq("build", "main.o");
        assert_eq!(dag.default_target.as_deref(), Some("build"));
    }

    #[test]
    fn test_dag_topo_order() {
        let mut dag = Dag::new();
        dag.add_prereq("all", "b");
        dag.add_prereq("b", "c");
        dag.add_prereq("c", "d");
        let order = dag.order("all").unwrap();
        let names: Vec<&str> = order.iter().map(|n| n.target.as_str()).collect();
        // d must come before c, c before b, b before all
        assert_eq!(names, vec!["d", "c", "b", "all"]);
    }

    #[test]
    fn test_dag_topo_order_diamond() {
        let mut dag = Dag::new();
        dag.add_prereq("all", "left");
        dag.add_prereq("all", "right");
        dag.add_prereq("left", "base");
        dag.add_prereq("right", "base");
        let order = dag.order("all").unwrap();
        let names: Vec<&str> = order.iter().map(|n| n.target.as_str()).collect();
        // base must be first
        assert_eq!(names[0], "base");
        assert_eq!(names.last().copied(), Some("all"));
        assert_eq!(names.len(), 4);
    }

    // -- Makefile parser --

    #[test]
    fn test_parse_simple_rule() {
        let content = "all: output\n\techo done\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert!(dag.nodes.contains_key("all"));
        assert_eq!(dag.nodes["all"].prereqs, vec!["output"]);
        assert_eq!(dag.nodes["all"].recipes, vec!["echo done"]);
    }

    #[test]
    fn test_parse_multiple_prereqs() {
        let content = "program: main.o utils.o\n\tcat $^ > program\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        let prereqs = &dag.nodes["program"].prereqs;
        assert_eq!(prereqs.len(), 2);
        assert!(prereqs.contains(&"main.o".to_string()));
        assert!(prereqs.contains(&"utils.o".to_string()));
    }

    #[test]
    fn test_parse_phony() {
        let content = ".PHONY: all clean\nall: output\n\tcat output\nclean:\n\trm -f output\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert!(dag.nodes[".PHONY"].is_phony);
        assert!(dag.nodes["all"].is_phony);
        assert!(dag.nodes["clean"].is_phony);
    }

    #[test]
    fn test_parse_order_only_prereqs() {
        let content = "all: out1 out2 | dir1 dir2\n\techo done\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        let node = &dag.nodes["all"];
        assert_eq!(node.prereqs, vec!["out1", "out2"]);
        assert_eq!(node.order_prereqs, vec!["dir1", "dir2"]);
    }

    #[test]
    fn test_parse_comments_skipped() {
        let content = "# this is a comment\nall: output\n\techo\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert!(!dag.nodes.contains_key("# this is a comment"));
        assert!(dag.nodes.contains_key("all"));
    }

    #[test]
    fn test_parse_variable_assignment_ignored() {
        let content = "CC = gcc\nall: main.o\n\t$(CC) -o all main.o\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        // CC = gcc should not become a rule
        assert!(!dag.nodes.contains_key("CC = gcc"));
        assert!(dag.nodes.contains_key("all"));
    }

    #[test]
    fn test_parse_empty_lines_reset_current_target() {
        let content = "a: b\n\techo a\n\nb:\n\techo b\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert_eq!(dag.nodes["a"].recipes, vec!["echo a"]);
        assert_eq!(dag.nodes["b"].recipes, vec!["echo b"]);
    }

    #[test]
    fn test_parse_no_recipe_target() {
        let content = "all: deps\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert!(dag.nodes["all"].recipes.is_empty());
    }

    // -- Automatic variable expansion --

    #[test]
    fn test_expand_at_target() {
        let node = DagNode::new("output.txt".to_string(), false);
        assert_eq!(expand_vars("$@", &node, &HashMap::new()), "output.txt");
    }

    #[test]
    fn test_expand_less_first_prereq() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("first.o".to_string());
        node.prereqs.push("second.o".to_string());
        assert_eq!(expand_vars("$<", &node, &HashMap::new()), "first.o");
    }

    #[test]
    fn test_expand_hat_all_prereqs() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("a.o".to_string());
        node.prereqs.push("b.o".to_string());
        assert_eq!(expand_vars("$^", &node, &HashMap::new()), "a.o b.o");
    }

    #[test]
    fn test_expand_dollar_sign() {
        let node = DagNode::new("x".to_string(), false);
        assert_eq!(expand_vars("$$", &node, &HashMap::new()), "$");
    }

    #[test]
    fn test_expand_mixed() {
        let mut node = DagNode::new("hello".to_string(), false);
        node.prereqs.push("hello.o".to_string());
        assert_eq!(
            expand_vars("gcc -o $@ $<", &node, &HashMap::new()),
            "gcc -o hello hello.o"
        );
    }

    #[test]
    fn test_expand_unknown_var_passes_through() {
        let node = DagNode::new("x".to_string(), false);
        // $* (stem) is now implemented, should return empty when no stem set
        assert_eq!(expand_vars("$*", &node, &HashMap::new()), "");
        // $? is now implemented
        assert_eq!(expand_vars("$?", &node, &HashMap::new()), "");
        // $| is now implemented
        assert_eq!(expand_vars("$|", &node, &HashMap::new()), "");
        // unknown vars now return empty string (variable lookup)
        assert_eq!(expand_vars("$X", &node, &HashMap::new()), "");
        assert_eq!(expand_vars("$(XXX)", &node, &HashMap::new()), "");
        // $$ escapes to $
        assert_eq!(expand_vars("$$", &node, &HashMap::new()), "$");
    }

    #[test]
    fn test_expand_dollar_plus() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("a.o".to_string());
        node.prereqs.push("b.o".to_string());
        node.prereqs.push("a.o".to_string());
        assert_eq!(expand_vars("$+", &node, &HashMap::new()), "a.o b.o a.o");
    }

    #[test]
    fn test_expand_stem() {
        let mut node = DagNode::new("foo.bar".to_string(), false);
        node.stem = Some("foo".to_string());
        assert_eq!(expand_vars("$*", &node, &HashMap::new()), "foo");
        assert_eq!(expand_vars("file: $*", &node, &HashMap::new()), "file: foo");
    }

    #[test]
    fn test_expand_order_prereqs() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("normal.o".to_string());
        node.order_prereqs.push("dir".to_string());
        assert_eq!(expand_vars("$|", &node, &HashMap::new()), "dir");
    }

    #[test]
    fn test_expand_dir_file_parts() {
        let mut node = DagNode::new("src/foo/bar.o".to_string(), false);
        node.prereqs.push("src/foo/bar.c".to_string());
        node.stem = Some("src/foo/bar".to_string());

        assert_eq!(expand_vars("$(@D)", &node, &HashMap::new()), "src/foo");
        assert_eq!(expand_vars("$(@F)", &node, &HashMap::new()), "bar.o");
        assert_eq!(expand_vars("$(<D)", &node, &HashMap::new()), "src/foo");
        assert_eq!(expand_vars("$(<F)", &node, &HashMap::new()), "bar.c");
        assert_eq!(expand_vars("$(*D)", &node, &HashMap::new()), "src/foo");
        assert_eq!(expand_vars("$(*F)", &node, &HashMap::new()), "bar");
    }

    #[test]
    fn test_expand_dir_file_root() {
        let mut node = DagNode::new("foo.o".to_string(), false);
        node.prereqs.push("foo.c".to_string());

        assert_eq!(expand_vars("$(@D)", &node, &HashMap::new()), ".");
        assert_eq!(expand_vars("$(@F)", &node, &HashMap::new()), "foo.o");
        assert_eq!(expand_vars("$(<D)", &node, &HashMap::new()), ".");
        assert_eq!(expand_vars("$(<F)", &node, &HashMap::new()), "foo.c");
    }

    #[test]
    fn test_expand_trailing_dollar() {
        let node = DagNode::new("x".to_string(), false);
        assert_eq!(expand_vars("foo$", &node, &HashMap::new()), "foo$");
    }

    #[test]
    fn test_expand_combined_vars() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("a".to_string());
        node.prereqs.push("b".to_string());
        node.order_prereqs.push("dir".to_string());
        node.stem = Some("foo".to_string());

        assert_eq!(
            expand_vars("@=$@ <=$< >=$<", &node, &HashMap::new()),
            "@=out <=a >=a"
        );
        assert_eq!(
            expand_vars("^=$^ +=$+", &node, &HashMap::new()),
            "^=a b +=a b"
        );
    }

    // -- Variable functions --

    #[test]
    fn test_expand_subst() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "subst",
                "foo,bar,foo foo foo",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "bar bar bar"
        );
    }

    #[test]
    fn test_expand_patsubst() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "patsubst",
                "%.o,%.c,foo.o bar.o",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo.c bar.c"
        );
    }

    #[test]
    fn test_expand_dir() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "dir",
                "src/foo.c bar.c",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "src ."
        );
    }

    #[test]
    fn test_expand_notdir() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "notdir",
                "src/foo.c bar.c",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo.c bar.c"
        );
    }

    #[test]
    fn test_expand_suffix() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "suffix",
                "foo.c bar.o baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            ".c .o"
        );
    }

    #[test]
    fn test_expand_basename() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "basename",
                "foo.c bar.o baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo bar baz"
        );
    }

    #[test]
    fn test_expand_addsuffix() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "addsuffix",
                ".c,foo bar",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo.c bar.c"
        );
    }

    #[test]
    fn test_expand_addprefix() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "addprefix",
                "src/,foo bar",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "src/foo src/bar"
        );
    }

    #[test]
    fn test_expand_filter() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "filter",
                "*.c *.h,foo.c bar.h baz.o",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo.c bar.h"
        );
    }

    #[test]
    fn test_expand_sort() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function("sort", "c b a b c", &DagNode::new("x".into(), false), &vars),
            "a b c"
        );
    }

    #[test]
    fn test_expand_word() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "word",
                "2,foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "bar"
        );
    }

    #[test]
    fn test_expand_words() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "words",
                "foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "3"
        );
    }

    #[test]
    fn test_expand_firstword() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "firstword",
                "foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo"
        );
    }

    #[test]
    fn test_expand_lastword() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "lastword",
                "foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "baz"
        );
    }

    #[test]
    fn test_expand_strip() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "strip",
                "  foo   bar  ",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo bar"
        );
    }

    #[test]
    fn test_expand_findstring() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "findstring",
                "foo,foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "foo"
        );
        assert_eq!(
            expand_function(
                "findstring",
                "qux,foo bar baz",
                &DagNode::new("x".into(), false),
                &vars
            ),
            ""
        );
    }

    #[test]
    fn test_expand_join() {
        let vars = HashMap::new();
        assert_eq!(
            expand_function(
                "join",
                "a b c,1 2 3",
                &DagNode::new("x".into(), false),
                &vars
            ),
            "a1 b2 c3"
        );
    }

    #[test]
    fn test_expand_variable_lookup() {
        let mut vars = HashMap::new();
        vars.insert("CC".to_string(), "gcc".to_string());
        vars.insert("CFLAGS".to_string(), "-Wall".to_string());
        let node = DagNode::new("x".into(), false);
        assert_eq!(expand_vars("$(CC) $(CFLAGS)", &node, &vars), "gcc -Wall");
    }

    #[test]
    fn test_parse_variables() {
        let content = "CC = gcc\nCFLAGS = -Wall\nall: main.o\n\t$(CC) $(CFLAGS) -o $@ $<\n";
        let mut dag = Dag::new();
        parse_makefile(content, &mut dag).unwrap();
        assert_eq!(dag.variables.get("CC").map(|s| s.as_str()), Some("gcc"));
        assert_eq!(
            dag.variables.get("CFLAGS").map(|s| s.as_str()),
            Some("-Wall")
        );
    }

    // -- Staleness --

    #[test]
    fn test_phony_always_rebuilds() {
        let node = DagNode::new("clean".to_string(), true);
        assert!(node.needs_rebuild());
    }

    #[test]
    fn test_missing_target_rebuilds() {
        let node = DagNode::new("nonexistent".to_string(), false);
        assert!(node.needs_rebuild());
    }

    #[test]
    fn test_missing_prereq_rebuilds() {
        let mut node = DagNode::new("target".to_string(), false);
        node.prereqs.push("nonexistent_prereq".to_string());
        assert!(node.needs_rebuild());
    }

    // -- Executor (dry run) --

    #[test]
    fn test_executor_dry_run() {
        let mut node = DagNode::new("all".to_string(), true);
        node.recipes.push("echo hello".to_string());
        node.recipes.push("@echo silent".to_string());

        let mut exec = Executor::new(false, true, HashMap::new());
        let ok = exec.run(&[&node]);
        assert!(ok);
        assert_eq!(exec.errors, 0);
    }

    #[test]
    fn test_executor_dry_run_no_at() {
        let mut node = DagNode::new("all".to_string(), true);
        node.recipes.push("@echo silent".to_string());

        let mut exec = Executor::new(false, true, HashMap::new());
        let ok = exec.run(&[&node]);
        assert!(ok);
    }

    // -- Cycle detection edge cases --

    #[test]
    fn test_no_cycles_single_node() {
        let mut dag = Dag::new();
        dag.add_prereq("a", "b");
        assert!(dag.detect_cycle().is_none());
    }

    #[test]
    fn test_self_cycle() {
        let mut dag = Dag::new();
        dag.add_prereq("a", "a");
        let cycle = dag.detect_cycle();
        assert!(cycle.is_some());
    }

    #[test]
    fn test_dag_disconnected_components_no_cycle() {
        let mut dag = Dag::new();
        dag.add_prereq("a", "b");
        dag.add_prereq("c", "d");
        assert!(dag.detect_cycle().is_none());
    }

    // -- Topo sort with missing nodes --

    #[test]
    fn test_topo_missing_prereq_still_works() {
        let mut dag = Dag::new();
        dag.add_prereq("all", "phantom");
        // "phantom" gets an empty node
        let order = dag.order("all").unwrap();
        assert_eq!(order.len(), 2);
        assert_eq!(order[0].target, "phantom");
        assert_eq!(order[1].target, "all");
    }
}
