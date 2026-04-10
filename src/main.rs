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
    recipes: Vec<String>,
    is_phony: bool,
}

impl DagNode {
    fn new(target: String, is_phony: bool) -> Self {
        Self {
            target,
            prereqs: Vec::new(),
            recipes: Vec::new(),
            is_phony,
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
    default_target: Option<String>,
}

impl Dag {
    fn new() -> Self {
        Self {
            nodes: HashMap::new(),
            default_target: None,
        }
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
        // If this is the first non-dot target, set as default
        if self.default_target.is_none()
            && !target.starts_with('.')
            && target != ".PHONY"
            && target != ".SUFFIXES"
        {
            self.set_default(target);
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

        // Look for `target: prereqs` (rule line, not a variable assignment)
        if let Some(colon_pos) = trimmed.find(':') {
            // Make sure it's not a variable assignment `:=` or `::=` etc.
            let after_colon = &trimmed[colon_pos + 1..];
            if after_colon.starts_with('=') {
                // This is a variable assignment, skip for now
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
                for prereq in prereqs_part.split_whitespace() {
                    dag.add_prereq(target_part, prereq);
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

fn expand_auto_vars(cmd: &str, node: &DagNode) -> String {
    let mut out = String::new();
    let chars: Vec<char> = cmd.chars().collect();
    let mut i = 0;
    while i < chars.len() {
        if chars[i] == '$' {
            if i + 1 < chars.len() {
                match chars[i + 1] {
                    '@' => {
                        out.push_str(&node.target);
                        i += 2;
                    }
                    '<' => {
                        if let Some(first) = node.prereqs.first() {
                            out.push_str(first);
                        }
                        i += 2;
                    }
                    '^' => {
                        out.push_str(&node.prereqs.join(" "));
                        i += 2;
                    }
                    '*' => {
                        // Stem — not fully implemented, skip for now
                        i += 2;
                    }
                    '$' => {
                        out.push('$');
                        i += 2;
                    }
                    _ => {
                        out.push('$');
                        i += 1;
                    }
                }
            } else {
                out.push('$');
                i += 1;
            }
        } else {
            out.push(chars[i]);
            i += 1;
        }
    }
    out
}

// ---------------------------------------------------------------------------
// Executor
// ---------------------------------------------------------------------------

struct Executor {
    keep_going: bool,
    dry_run: bool,
    errors: usize,
}

impl Executor {
    fn new(keep_going: bool, dry_run: bool) -> Self {
        Self {
            keep_going,
            dry_run,
            errors: 0,
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
            let expanded = expand_auto_vars(cmd, nd);

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
    let mut executor = Executor::new(args.keep_going, args.dry_run);
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
        assert_eq!(expand_auto_vars("$@", &node), "output.txt");
    }

    #[test]
    fn test_expand_less_first_prereq() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("first.o".to_string());
        node.prereqs.push("second.o".to_string());
        assert_eq!(expand_auto_vars("$<", &node), "first.o");
    }

    #[test]
    fn test_expand_hat_all_prereqs() {
        let mut node = DagNode::new("out".to_string(), false);
        node.prereqs.push("a.o".to_string());
        node.prereqs.push("b.o".to_string());
        assert_eq!(expand_auto_vars("$^", &node), "a.o b.o");
    }

    #[test]
    fn test_expand_dollar_sign() {
        let node = DagNode::new("x".to_string(), false);
        assert_eq!(expand_auto_vars("$$", &node), "$");
    }

    #[test]
    fn test_expand_mixed() {
        let mut node = DagNode::new("hello".to_string(), false);
        node.prereqs.push("hello.o".to_string());
        assert_eq!(
            expand_auto_vars("gcc -o $@ $<", &node),
            "gcc -o hello hello.o"
        );
    }

    #[test]
    fn test_expand_unknown_var_passes_through() {
        let node = DagNode::new("x".to_string(), false);
        // $* (stem) is not implemented, should be consumed without output
        assert_eq!(expand_auto_vars("$*", &node), "");
        // $? is unknown, should pass through literally as $
        assert_eq!(expand_auto_vars("$?", &node), "$?");
    }

    #[test]
    fn test_expand_trailing_dollar() {
        let node = DagNode::new("x".to_string(), false);
        assert_eq!(expand_auto_vars("foo$", &node), "foo$");
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

        let mut exec = Executor::new(false, true);
        let ok = exec.run(&[&node]);
        assert!(ok);
        assert_eq!(exec.errors, 0);
    }

    #[test]
    fn test_executor_dry_run_no_at() {
        let mut node = DagNode::new("all".to_string(), true);
        node.recipes.push("@echo silent".to_string());

        let mut exec = Executor::new(false, true);
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
