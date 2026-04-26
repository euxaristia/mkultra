use "collections"
use "files"

use @system[I32](command: Pointer[U8] tag)

primitive Stat
  fun mtime(auth: FileAuth, path: String): I64 =>
    """
    Returns mtime in seconds since epoch, or 0 if missing.
    """
    try
      let fp = FilePath(auth, path)
      let info = FileInfo(fp)?
      info.modified_time._1
    else
      I64(0)
    end

  fun needs_rebuild(node: DagNode box, auth: FileAuth): Bool =>
    if node.is_phony then return true end
    let target_mtime = mtime(auth, node.target)
    if target_mtime == 0 then return true end
    for prereq in node.prereqs.values() do
      let pm = mtime(auth, prereq)
      if (pm == 0) or (pm > target_mtime) then return true end
    end
    false

primitive ShellExec
  fun run(cmd: String): I32 =>
    """
    Run a shell command via libc system(3). Returns the exit code (0..255)
    for normal exits. Signals/abnormal returns produce status >= 128.
    """
    let raw = @system(cmd.cstring())
    if raw == -1 then 127
    elseif (raw and 0x7F) == 0 then  // normal exit
      (raw >> 8) and 0xFF
    else
      128 + (raw and 0x7F)
    end

class val Job
  let target: String
  let recipes: Array[String] val
  let prereqs: Array[String] val
  let is_phony: Bool

  new val create(target': String, recipes': Array[String] val,
    prereqs': Array[String] val, is_phony': Bool)
  =>
    target = target'
    recipes = recipes'
    prereqs = prereqs'
    is_phony = is_phony'

actor Executor
  let _jobs: USize
  let _keep_going: Bool
  let _ignore_errors: Bool
  let _silent: Bool
  let _dry_run: Bool
  let _vars: Map[String, String] val
  let _auth: FileAuth
  let _out: OutStream
  let _err: OutStream
  let _env: Env
  let _target: String

  let _by_name: Map[String, Job] = Map[String, Job]
  let _remaining: Map[String, USize] = Map[String, USize]
  let _dependents: Map[String, Array[String]] = Map[String, Array[String]]
  let _ready: Array[String] = Array[String]
  let _completed: Set[String] = Set[String]
  let _failed: Set[String] = Set[String]
  var _in_flight: USize = 0
  var _errors: USize = 0
  var _stop_dispatch: Bool = false
  var _needs_build: Bool = false
  var _started: Bool = false

  let _touch: Bool

  new create(jobs: USize, keep_going: Bool, ignore_errors: Bool,
    silent: Bool, dry_run: Bool, touch: Bool,
    vars: Map[String, String] val,
    auth: FileAuth, out: OutStream, err: OutStream, env: Env,
    target: String)
  =>
    _jobs = if jobs > 0 then jobs else 1 end
    _keep_going = keep_going
    _ignore_errors = ignore_errors
    _silent = silent
    _dry_run = dry_run
    _touch = touch
    _vars = vars
    _auth = auth
    _out = out
    _err = err
    _env = env
    _target = target

  be start(jobs_list: Array[Job] val) =>
    if _started then return end
    _started = true

    let in_set = Set[String]
    for j in jobs_list.values() do in_set.set(j.target) end

    for j in jobs_list.values() do
      _by_name(j.target) = j
      var count: USize = 0
      for p in j.prereqs.values() do
        if in_set.contains(p) then
          count = count + 1
          let arr =
            try
              _dependents(p)?
            else
              let a = Array[String]
              _dependents(p) = a
              a
            end
          arr.push(j.target)
        end
      end
      _remaining(j.target) = count
    end

    for j in jobs_list.values() do
      if (j.recipes.size() > 0) and _job_needs_rebuild(j) then
        _needs_build = true
        break
      end
    end

    if not _needs_build then
      _out.print("mkultra: nothing to be done for '" + _target + "'")
      return
    end

    let initial_roots = Array[String]
    for j in jobs_list.values() do
      if (try _remaining(j.target)? else 0 end) == 0 then
        initial_roots.push(j.target)
      end
    end
    for name in initial_roots.values() do
      _enqueue(name)
    end

    _dispatch()
    _maybe_finalize()

  be _job_done(target: String, ok: Bool) =>
    _in_flight = _in_flight - 1
    if not ok then
      _errors = _errors + 1
      _failed.set(target)
      if not (_keep_going or _ignore_errors) then
        _stop_dispatch = true
      end
    end
    _complete(target)
    _dispatch()
    _maybe_finalize()

  fun ref _enqueue(target: String) =>
    if _completed.contains(target) then return end
    try
      let job = _by_name(target)?

      // If any prereq has failed, this target can't be built. Mark it
      // failed and propagate, so dependents skip too. Important under -k.
      for p in job.prereqs.values() do
        if _failed.contains(p) then
          _failed.set(target)
          if job.recipes.size() > 0 then
            _err.print("mkultra: Target '" + target
              + "' not remade because of errors")
          end
          _complete(target)
          return
        end
      end

      if (job.recipes.size() == 0) or (not _job_needs_rebuild(job)) then
        _complete(target)
      else
        _ready.push(target)
      end
    end

  fun ref _complete(target: String) =>
    if _completed.contains(target) then return end
    _completed.set(target)
    try
      let deps = _dependents(target)?
      for dep in deps.values() do
        let cur = try _remaining(dep)? else 0 end
        if cur > 0 then
          let next = cur - 1
          _remaining(dep) = next
          if (next == 0) and (not _stop_dispatch) then
            _enqueue(dep)
          end
        end
      end
    end

  fun ref _dispatch() =>
    if _stop_dispatch then return end
    while (_in_flight < _jobs) and (_ready.size() > 0) do
      try
        let target = _ready.shift()?
        let job = _by_name(target)?
        let worker = _Worker(this, _vars, _auth, _out, _err,
          _silent, _dry_run, _ignore_errors, _touch)
        worker.run(job)
        _in_flight = _in_flight + 1
      end
    end

  fun ref _maybe_finalize() =>
    // In stop-dispatch mode, _ready may still hold queued work that will
    // never run. Finalize once in-flight reaches 0 regardless.
    if _in_flight > 0 then return end
    if _stop_dispatch or (_ready.size() == 0) then _finalize() end

  fun ref _finalize() =>
    if _errors > 0 then _env.exitcode(1) end

  fun _job_needs_rebuild(j: Job): Bool =>
    if j.is_phony then return true end
    let target_mtime = Stat.mtime(_auth, j.target)
    if target_mtime == 0 then return true end
    for prereq in j.prereqs.values() do
      let pm = Stat.mtime(_auth, prereq)
      if (pm == 0) or (pm > target_mtime) then return true end
    end
    false

actor _Worker
  let _coord: Executor tag
  let _vars: Map[String, String] val
  let _auth: FileAuth
  let _out: OutStream
  let _err: OutStream
  let _silent: Bool
  let _dry_run: Bool
  let _ignore_errors: Bool
  let _touch: Bool

  new create(coord: Executor tag, vars: Map[String, String] val,
    auth: FileAuth, out: OutStream, err: OutStream,
    silent: Bool, dry_run: Bool, ignore_errors: Bool, touch: Bool)
  =>
    _coord = coord
    _vars = vars
    _auth = auth
    _out = out
    _err = err
    _silent = silent
    _dry_run = dry_run
    _ignore_errors = ignore_errors
    _touch = touch

  be run(job: Job) =>
    if _touch then
      // -t: replace recipes with `touch <target>` (skip phony targets).
      if not job.is_phony then
        let cmd: String val = "touch " + job.target
        if not _silent then _out.print(cmd) end
        ShellExec.run(cmd)
      end
      _coord._job_done(job.target, true)
      return
    end

    let auto = AutoVars(job.target, job.prereqs)
    var ok = true
    for recipe in job.recipes.values() do
      (let cmd_raw, let silent_pref, let ignore_pref, let always_pref) =
        _parse_prefixes(recipe)
      let suppressed = silent_pref or _silent
      let expanded: String val = Expand.with_auto(cmd_raw, auto, _vars, _auth)

      // `-n` skips execution, but `+`-prefixed recipes always run (POSIX).
      if _dry_run and (not always_pref) then
        if not suppressed then _out.print(expanded) end
        continue
      end
      if not suppressed then _out.print(expanded) end

      let code = ShellExec.run(expanded)
      if code != 0 then
        if ignore_pref or _ignore_errors then
          _err.print("mkultra: [" + job.target + "] Error " + code.string()
            + " (ignored)")
          continue
        end
        _err.print("mkultra: *** [" + job.target + "] Error " + code.string())
        ok = false
        break
      end
    end
    _coord._job_done(job.target, ok)

  fun _parse_prefixes(recipe: String): (String, Bool, Bool, Bool) =>
    """
    Strip leading `@`, `-`, `+` prefixes (any order, any combination)
    and return (command, silent, ignore_err, always_run).
    """
    var silent = false
    var ignore_err = false
    var always_run = false
    var i: USize = 0
    let n = recipe.size()
    try
      while i < n do
        let c = recipe(i)?
        if c == '@' then silent = true
        elseif c == '-' then ignore_err = true
        elseif c == '+' then always_run = true
        else break
        end
        i = i + 1
      end
    end
    (recipe.substring(i.isize()), silent, ignore_err, always_run)
