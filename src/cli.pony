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
  var help: Bool = false
  var version: Bool = false

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
        | "-i" => a.ignore_errors = true
        | "-s" => a.silent = true
        | "-q" => a.question = true
        | "-p" => a.print_db = true
        | "-n" => a.dry_run = true
        | "-r" => None // no-op
        | "-h" => a.help = true
        | "--help" => a.help = true
        | "--version" => a.version = true
        else
          if (s.size() > 0) and (try s(0)? == '-' else false end) then
            return "unknown option: " + s
          end
          a.target = s
        end
        i = i + 1
      end
    end
    a

primitive Usage
  fun print(out: OutStream) =>
    out.print("Usage: mkultra [target] [-f FILE] [-j N] [-iknpqrs]")
    out.print("")
    out.print("Options:")
    out.print("  -f FILE   Read FILE as the makefile (default: Makefile, then makefile)")
    out.print("  -j N      Run up to N recipes in parallel (default: 1)")
    out.print("  -i        Ignore errors from commands")
    out.print("  -k        Keep going after errors")
    out.print("  -n        Dry run (print commands but don't execute)")
    out.print("  -p        Print database (rules and variables)")
    out.print("  -q        Question mode (exit 0 if up to date, 1 otherwise)")
    out.print("  -r        Disable built-in rules (no-op for compatibility)")
    out.print("  -s        Silent mode (don't echo commands)")
    out.print("  -h        Show this help")
    out.print("  --version Show version")
