use "collections"
use "files"

use @popen[Pointer[U8]](command: Pointer[U8] tag, mode: Pointer[U8] tag)
use @pclose[I32](fp: Pointer[U8])
use @fread[USize](buf: Pointer[U8] tag, size: USize, count: USize, fp: Pointer[U8])

primitive Chars
  fun is_var_char(c: U8): Bool =>
    ((c >= '0') and (c <= '9')) or
    ((c >= 'a') and (c <= 'z')) or
    ((c >= 'A') and (c <= 'Z')) or
    (c == '_')

  fun is_ws(c: U8): Bool =>
    (c == ' ') or (c == '\t') or (c == '\n') or (c == '\r')

primitive Strs
  fun trim(s: String box): String =>
    var start: USize = 0
    var endp: USize = s.size()
    try
      while (start < endp) and Chars.is_ws(s(start)?) do start = start + 1 end
      while (endp > start) and Chars.is_ws(s(endp - 1)?) do endp = endp - 1 end
    end
    s.substring(start.isize(), endp.isize())

  fun split_ws(s: String box): Array[String] =>
    let out = Array[String]
    var i: USize = 0
    let n = s.size()
    try
      while i < n do
        while (i < n) and Chars.is_ws(s(i)?) do i = i + 1 end
        if i >= n then break end
        let start = i
        while (i < n) and (not Chars.is_ws(s(i)?)) do i = i + 1 end
        out.push(s.substring(start.isize(), i.isize()))
      end
    end
    out

primitive Glob
  fun matches(pattern: String, name: String): Bool =>
    _matches(pattern, 0, name, 0)

  fun _matches(p: String, pi: USize, n: String, ni: USize): Bool =>
    if pi >= p.size() then return ni >= n.size() end
    let pc = try p(pi)? else return false end
    if pc == '*' then
      var i = ni
      while i <= n.size() do
        if _matches(p, pi + 1, n, i) then return true end
        i = i + 1
      end
      false
    elseif pc == '?' then
      if ni >= n.size() then return false end
      _matches(p, pi + 1, n, ni + 1)
    else
      if ni >= n.size() then return false end
      let nc = try n(ni)? else return false end
      if pc != nc then return false end
      _matches(p, pi + 1, n, ni + 1)
    end

primitive Shell
  fun capture(cmd: String): String =>
    let cmd_cstr = cmd.cstring()
    let mode = "r"
    let fp = @popen(cmd_cstr, mode.cstring())
    if fp.is_null() then return "" end
    let buf = String
    let chunk_size: USize = 4096
    let chunk = Array[U8].init(0, chunk_size)
    var done = false
    while not done do
      let got = @fread(chunk.cpointer(), USize(1), chunk_size, fp)
      if got == 0 then
        done = true
      else
        var i: USize = 0
        try
          while i < got do
            buf.push(chunk(i)?)
            i = i + 1
          end
        end
      end
    end
    @pclose(fp)
    // Collapse whitespace runs into single spaces, like the Rust version
    " ".join(Strs.split_ws(buf).values())

primitive Wildcard
  fun expand(auth: FileAuth, pattern: String): String =>
    if (not pattern.contains("*")) and (not pattern.contains("?")) then
      return pattern
    end
    var dir = "."
    var name_pat = pattern
    try
      let slash = pattern.rfind("/")?
      dir = pattern.substring(0, slash)
      name_pat = pattern.substring(slash + 1)
      if dir.size() == 0 then dir = "/" end
    end
    try
      let path = FilePath(auth, dir)
      let d = Directory(path)?
      let entries: Array[String] ref = d.entries()?
      let matches = Array[String]
      for e in entries.values() do
        if Glob.matches(name_pat, e) then
          let full: String val = if dir == "." then e else dir + "/" + e end
          matches.push(full)
        end
      end
      " ".join(matches.values())
    else
      ""
    end

class _ParsedVar
  let name: String
  let consumed: USize
  new create(name': String, consumed': USize) =>
    name = name'
    consumed = consumed'

class _ParsedFunc
  let name: String
  let args: String
  let consumed: USize
  new create(name': String, args': String, consumed': USize) =>
    name = name'
    args = args'
    consumed = consumed'

primitive Expand
  fun simple(text: String, vars: Map[String, String] box,
    auth: FileAuth): String
  =>
    """
    Expand variables and functions in `text` without auto-vars (used for
    parse-time expansion of targets, prereqs, and := RHS).
    """
    _expand(text, None, vars, auth)

  fun with_node(text: String, node: DagNode box,
    vars: Map[String, String] box, auth: FileAuth): String
  =>
    """
    Expand variables, functions, and auto-vars ($@/$</$^) for recipes.
    """
    _expand(text, node, vars, auth)

  fun _expand(text: String, node: (DagNode box | None),
    vars: Map[String, String] box, auth: FileAuth): String
  =>
    let out = recover ref String end
    var i: USize = 0
    let n = text.size()
    try
      while i < n do
        let c = text(i)?
        if c != '$' then
          out.push(c)
          i = i + 1
          continue
        end
        if (i + 1) >= n then
          out.push('$')
          i = i + 1
          continue
        end
        let nc = text(i + 1)?
        if nc == '$' then
          out.push('$')
          i = i + 2
          continue
        end
        if nc == '(' then
          match _parse_func(text, i + 1)
          | let f: _ParsedFunc =>
            out.append(_call(f.name, f.args, node, vars, auth))
            i = i + f.consumed
            continue
          end
        end
        // Auto vars only when node is provided
        match node
        | let nd: DagNode box =>
          if nc == '@' then
            out.append(nd.target)
            i = i + 2
            continue
          end
          if nc == '<' then
            try out.append(nd.prereqs(0)?) end
            i = i + 2
            continue
          end
          if nc == '^' then
            out.append(" ".join(nd.prereqs.values()))
            i = i + 2
            continue
          end
        end
        if Chars.is_var_char(nc) then
          match _parse_simple(text, i + 1)
          | let v: _ParsedVar =>
            let raw = try vars(v.name)? else "" end
            out.append(_expand(raw, node, vars, auth))
            i = i + v.consumed
            continue
          end
        end
        out.push('$')
        i = i + 1
      end
    end
    out.clone()

  fun _parse_simple(text: String, start: USize): (_ParsedVar | None) =>
    var endp = start
    try
      while (endp < text.size()) and Chars.is_var_char(text(endp)?) do
        endp = endp + 1
      end
    end
    if endp > start then
      _ParsedVar(text.substring(start.isize(), endp.isize()),
        1 + (endp - start))
    else
      None
    end

  fun _parse_func(text: String, start: USize): (_ParsedFunc | None) =>
    // text(start) is '(', find matching ')'
    var depth: USize = 1
    var i: USize = start + 1
    let n = text.size()
    try
      while (i < n) and (depth > 0) do
        let c = text(i)?
        if c == '(' then depth = depth + 1
        elseif c == ')' then depth = depth - 1
        end
        i = i + 1
      end
    end
    if depth != 0 then return None end
    // full = text[start+1 .. i-1]  (excludes the closing ')')
    let full: String val = text.substring((start + 1).isize(), (i - 1).isize())
    let consumed = (i - start) + 1  // accounts for the leading '$'
    // Try to parse "name args" — name followed by space, or "name,args" or just "name"
    try
      let space_idx = full.find(" ")?
      let name: String val = full.substring(0, space_idx)
      let args: String val = full.substring(space_idx + 1)
      return _ParsedFunc(name, args, consumed)
    end
    try
      let comma_idx = full.find(",")?
      let name: String val = full.substring(0, comma_idx)
      let args: String val = full.substring(comma_idx + 1)
      return _ParsedFunc(name, args, consumed)
    end
    _ParsedFunc(full, "", consumed)

  fun _call(name: String, args: String, node: (DagNode box | None),
    vars: Map[String, String] box, auth: FileAuth): String
  =>
    match name
    | "wildcard" => Wildcard.expand(auth, Strs.trim(args))
    | "shell" => Shell.capture(Strs.trim(args))
    else
      // Default: treat as a variable lookup (for backward-compat with
      // patterns like $(VAR) which never have args).
      let raw = try vars(name)? else "" end
      _expand(raw, node, vars, auth)
    end
