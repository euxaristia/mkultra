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

class Executor
  let _keep_going: Bool
  let _ignore_errors: Bool
  let _silent: Bool
  let _dry_run: Bool
  let _question: Bool
  let _vars: Map[String, String] box
  let _auth: FileAuth
  let _out: OutStream
  let _err: OutStream
  var errors: USize = 0
  var needs_build: Bool = false

  new create(args: CliArgs box, vars: Map[String, String] box,
    auth: FileAuth, out: OutStream, err: OutStream)
  =>
    _keep_going = args.keep_going
    _ignore_errors = args.ignore_errors
    _silent = args.silent
    _dry_run = args.dry_run
    _question = args.question
    _vars = vars
    _auth = auth
    _out = out
    _err = err

  fun ref run(nodes: Array[DagNode] box, target: String): Bool =>
    for nd in nodes.values() do
      if not Stat.needs_rebuild(nd, _auth) then continue end
      if nd.recipes.size() == 0 then continue end
      needs_build = true
      if _question then return false end
      if (not _exec_node(nd)) and (not _keep_going) and (not _ignore_errors) then
        return false
      end
    end
    if (not needs_build) and (not _question) then
      _out.print("mkultra: nothing to be done for '" + target + "'")
    end
    errors == 0

  fun ref _exec_node(nd: DagNode box): Bool =>
    for recipe in nd.recipes.values() do
      let at_prefixed = (recipe.size() > 0) and (try recipe(0)? == '@' else false end)
      let suppressed = at_prefixed or _silent
      let cmd_raw: String val =
        if at_prefixed then recipe.substring(1) else recipe end
      let expanded: String val = Expand.with_node(cmd_raw, nd, _vars, _auth)

      if _dry_run then
        if not suppressed then _out.print(expanded) end
        continue
      end

      if not suppressed then _out.print(expanded) end

      let code = ShellExec.run(expanded)
      if code != 0 then
        errors = errors + 1
        _err.print("mkultra: *** [" + nd.target + "] Error " + code.string())
        if _ignore_errors then continue end
        return false
      end
    end
    true
