class CliArgs
  var target: (String | None) = None
  var makefile: (String | None) = None
  var jobs: (USize | None) = None
  var keep_going: Bool = false
  var ignore_errors: Bool = false
  var silent: Bool = false
  var question: Bool = false
  var print_db: Bool = false
  var dry_run: Bool = false
  var touch: Bool = false
  var env_override: Bool = false
  var help: Bool = false
  var version: Bool = false
  let overrides: Array[(String, String)] = Array[(String, String)]

  new create() => None

type ParseResult is (CliArgs | String)

primitive Cli
  fun parse(argv: Array[String] box): ParseResult =>
    let a = CliArgs
    var i: USize = 0
    let n = argv.size()
    try
      while i < n do
        let s = argv(i)?
        match s
        | "-f" =>
          i = i + 1
          if i >= n then return "-f requires an argument" end
          a.makefile = argv(i)?
        | "-j" =>
          i = i + 1
          if i >= n then return "-j requires an argument" end
          try a.jobs = argv(i)?.usize()? else return "invalid -j value" end
        | "-k" => a.keep_going = true
        | "-S" => a.keep_going = false
        | "-i" => a.ignore_errors = true
        | "-s" => a.silent = true
        | "-q" => a.question = true
        | "-p" => a.print_db = true
        | "-n" => a.dry_run = true
        | "-t" => a.touch = true
        | "-e" => a.env_override = true
        | "-r" => None // no-op
        | "-h" => a.help = true
        | "--help" => a.help = true
        | "--version" => a.version = true
        else
          if (s.size() > 0) and (try s(0)? == '-' else false end) then
            return "unknown option: " + s
          end
          // Macro override: NAME=value (POSIX command-line macro).
          var is_override = false
          try
            let eq_idx = s.find("=")?
            let name: String val = s.substring(0, eq_idx)
            if _is_valid_name(name) then
              let value: String val = s.substring(eq_idx + 1)
              a.overrides.push((name, value))
              is_override = true
            end
          end
          if not is_override then a.target = s end
        end
        i = i + 1
      end
    end
    a

  fun _is_valid_name(s: String): Bool =>
    if s.size() == 0 then return false end
    try
      let first = s(0)?
      if not (((first >= 'a') and (first <= 'z'))
          or ((first >= 'A') and (first <= 'Z'))
          or (first == '_'))
      then return false end
      var i: USize = 1
      while i < s.size() do
        let c = s(i)?
        if not (((c >= 'a') and (c <= 'z'))
            or ((c >= 'A') and (c <= 'Z'))
            or ((c >= '0') and (c <= '9'))
            or (c == '_'))
        then return false end
        i = i + 1
      end
      true
    else
      false
    end

primitive Usage
  fun print(out: OutStream) =>
    out.print("Usage: mkultra [target] [NAME=value ...] [-f FILE] [-j N] [-eikSnpqrst]")
    out.print("")
    out.print("Options:")
    out.print("  -f FILE   Read FILE as the makefile (default: Makefile, then makefile)")
    out.print("  -j N      Run up to N recipes in parallel (default: 1)")
    out.print("  -e        Environment variables override Makefile assignments")
    out.print("  -i        Ignore errors from commands")
    out.print("  -k        Keep going after errors")
    out.print("  -S        Cancel a prior -k (errors stop the build)")
    out.print("  -n        Dry run (print commands but don't execute)")
    out.print("  -p        Print database (rules and variables)")
    out.print("  -q        Question mode (exit 0 if up to date, 1 otherwise)")
    out.print("  -r        Disable built-in rules (no-op for compatibility)")
    out.print("  -s        Silent mode (don't echo commands)")
    out.print("  -t        Touch targets instead of running recipes")
    out.print("  -h        Show this help")
    out.print("  --version Show version")
