use "collections"
use "files"

actor Main
  new create(env: Env) =>
    // Build argv (skip program name)
    let argv = recover val
      let a = Array[String]
      let n = env.args.size()
      var i: USize = 1
      while i < n do
        try a.push(env.args(i)?) end
        i = i + 1
      end
      a
    end

    let args =
      match Cli.parse(argv)
      | let s: String =>
        env.err.print("mkultra: " + s)
        env.exitcode(2)
        return
      | let a: CliArgs => a
      end

    if args.help then
      Usage.print(env.out)
      return
    end

    if args.version then
      env.out.print("mkultra 0.2.0")
      return
    end

    let auth = FileAuth(env.root)

    // Resolve the makefile to use
    let makefile: String =
      match args.makefile
      | let m: String => m
      else
        if FileInfoExists(auth,"Makefile") then
          "Makefile"
        elseif FileInfoExists(auth,"makefile") then
          "makefile"
        else
          env.err.print("mkultra: *** No makefile found.")
          env.exitcode(2)
          return
        end
      end

    // Read the makefile
    let content =
      match OpenFile(FilePath(auth, makefile))
      | let file: File =>
        let size = file.size()
        let s: String val = file.read_string(size)
        s
      else
        env.err.print("mkultra: *** Cannot open " + makefile)
        env.exitcode(2)
        return
      end

    let dag: Dag ref = Dag

    // Import environment variables (matches GNU make's behavior)
    for kv in env.vars.values() do
      try
        let eq = kv.find("=")?.usize()
        let k: String val = kv.substring(0, eq.isize())
        let v: String val = kv.substring((eq + 1).isize())
        dag.set_variable(k, v)
      end
    end

    match Parser.parse(content, dag, auth)
    | let e: String =>
      env.err.print("mkultra: " + e)
      env.exitcode(2)
      return
    end

    // Cycle detection
    match dag.detect_cycle()
    | let cycle: Array[String] =>
      env.err.print("mkultra: *** Circular dependency: " + " -> ".join(cycle.values()))
      env.exitcode(2)
      return
    end

    // Determine target
    let build_target: String =
      match args.target
      | let t: String =>
        if not dag.nodes.contains(t) then
          env.err.print("mkultra: *** No rule to make " + t)
          env.exitcode(2)
          return
        end
        t
      else
        match dag.default_target
        | let dt: String => dt
        else
          env.err.print("mkultra: *** No default target.")
          env.exitcode(2)
          return
        end
      end

    let nodes = dag.order(build_target)

    // Snapshot variables as val so workers can share them.
    let vars_iso = recover iso Map[String, String] end
    for (k, v) in dag.variables.pairs() do vars_iso(k) = v end
    let vars_val: Map[String, String] val = consume vars_iso

    // -p: print database
    if args.print_db then
      env.out.print("# Variables")
      for (k, v) in dag.variables.pairs() do
        env.out.print(k + " = " + v)
      end
      env.out.print("")
      env.out.print("# Rules")
      for (name, node) in dag.nodes.pairs() do
        if (node.prereqs.size() > 0) or (node.recipes.size() > 0) then
          let line = String
          line.append(name)
          line.append(":")
          for p in node.prereqs.values() do
            line.append(" ")
            line.append(p)
          end
          env.out.print(line.clone())
          for r in node.recipes.values() do
            env.out.print("\t" + r)
          end
        end
      end
      return
    end

    // -q: question mode
    if args.question then
      for nd in nodes.values() do
        if Stat.needs_rebuild(nd, auth) then
          env.exitcode(1)
          return
        end
      end
      return
    end

    // Snapshot nodes as val Jobs so they can be sent to worker actors.
    let jobs_iso = recover iso Array[Job] end
    for nd in nodes.values() do
      let recipes_iso = recover iso Array[String] end
      for s in nd.recipes.values() do recipes_iso.push(s) end
      let prereqs_iso = recover iso Array[String] end
      for s in nd.prereqs.values() do prereqs_iso.push(s) end
      jobs_iso.push(Job(nd.target, consume recipes_iso, consume prereqs_iso,
        nd.is_phony))
    end
    let jobs_val: Array[Job] val = consume jobs_iso

    // Run
    let n_jobs: USize =
      match args.jobs
      | let n: USize => n
      else 1
      end
    let exec = Executor(n_jobs, args.keep_going, args.ignore_errors,
      args.silent, args.dry_run, vars_val, auth, env.out, env.err, env,
      build_target)
    exec.start(jobs_val)

primitive FileInfoExists
  fun apply(auth: FileAuth, path: String): Bool =>
    try
      FileInfo(FilePath(auth, path))?
      true
    else
      false
    end
