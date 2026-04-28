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

  fun is_identifier(s: String box): Bool =>
    """
    True iff s starts with [a-zA-Z_] and is followed by [a-zA-Z0-9_]*
    (POSIX macro name).
    """
    if s.size() == 0 then return false end
    try
      let first = s(0)?
      if not (((first >= 'a') and (first <= 'z'))
          or ((first >= 'A') and (first <= 'Z'))
          or (first == '_'))
      then return false end
      var i: USize = 1
      while i < s.size() do
        if not Chars.is_var_char(s(i)?) then return false end
        i = i + 1
      end
      true
    else false end

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
        buf.append(chunk, 0, got)
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

class val AutoVars
  let target: String
  let prereqs: Array[String] val
  new val create(target': String, prereqs': Array[String] val) =>
    target = target'
    prereqs = prereqs'

primitive Expand
  fun simple(text: String, vars: Map[String, String] box,
    auth: FileAuth): String
  =>
    """
    Expand variables and functions in `text` without auto-vars (used for
    parse-time expansion of targets, prereqs, and := RHS).
    """
    _expand(text, None, vars, auth, Set[String])

  fun with_auto(text: String, auto: AutoVars,
    vars: Map[String, String] box, auth: FileAuth): String
  =>
    """
    Expand variables, functions, and auto-vars ($@/$</$^) for recipes.
    """
    _expand(text, auto, vars, auth, Set[String])

  fun _expand(text: String, auto: (AutoVars | None),
    vars: Map[String, String] box, auth: FileAuth,
    expanding: Set[String]): String
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
        if (nc == '(') or (nc == '{') then
          match _parse_func(text, i + 1, nc)
          | let f: _ParsedFunc =>
            out.append(_call(f.name, f.args, auto, vars, auth, expanding))
            i = i + f.consumed
            continue
          end
        end
        // Auto vars only when target context is provided
        match auto
        | let av: AutoVars =>
          if nc == '@' then
            out.append(av.target)
            i = i + 2
            continue
          end
          if nc == '<' then
            try out.append(av.prereqs(0)?) end
            i = i + 2
            continue
          end
          if nc == '^' then
            // POSIX: deduplicated prereqs.
            let seen = Set[String]
            let dedup = Array[String]
            for p in av.prereqs.values() do
              if not seen.contains(p) then
                seen.set(p)
                dedup.push(p)
              end
            end
            out.append(" ".join(dedup.values()))
            i = i + 2
            continue
          end
          if nc == '+' then
            // POSIX: prereqs preserving duplicates.
            out.append(" ".join(av.prereqs.values()))
            i = i + 2
            continue
          end
          if nc == '?' then
            // POSIX: prereqs newer than the target (or all, if target absent).
            let target_mtime = Stat.mtime(auth, av.target)
            let newer = Array[String]
            for p in av.prereqs.values() do
              let pm = Stat.mtime(auth, p)
              if (target_mtime == 0) or (pm > target_mtime) then
                newer.push(p)
              end
            end
            out.append(" ".join(newer.values()))
            i = i + 2
            continue
          end
          if nc == '*' then
            // Stem — only meaningful inside suffix-rule context (future PR).
            // Consume to avoid emitting literal `$*`.
            i = i + 2
            continue
          end
        end
        if Chars.is_var_char(nc) then
          match _parse_simple(text, i + 1)
          | let v: _ParsedVar =>
            if not expanding.contains(v.name) then
              let raw = try vars(v.name)? else "" end
              expanding.set(v.name)
              out.append(_expand(raw, auto, vars, auth, expanding))
              expanding.unset(v.name)
            end
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

  fun _parse_func(text: String, start: USize, opener: U8): (_ParsedFunc | None) =>
    // text(start) is opener (`(` or `{`); find matching closer.
    let closer: U8 = if opener == '(' then ')' else '}' end
    var depth: USize = 1
    var i: USize = start + 1
    let n = text.size()
    try
      while (i < n) and (depth > 0) do
        let c = text(i)?
        if c == opener then depth = depth + 1
        elseif c == closer then depth = depth - 1
        end
        i = i + 1
      end
    end
    if depth != 0 then return None end
    // full = text[start+1 .. i-1]  (excludes the closing ')')
    let full: String val = text.substring((start + 1).isize(), (i - 1).isize())
    let consumed = (i - start) + 1  // accounts for the leading '$'

    // Substitution reference: `name:s1=s2` (whitespace around `name`
    // is tolerated). Keep the whole `full` as the name so _call can
    // route it through the substitution path. Match only when the
    // text before the first `:` trims to a valid identifier — that
    // way `$(shell echo "a:b=c")` doesn't get misclassified.
    try
      let colon_idx = full.find(":")?
      let prefix: String box = full.substring(0, colon_idx)
      if Strs.is_identifier(Strs.trim(prefix)) then
        let suffix: String box = full.substring(colon_idx + 1)
        suffix.find("=")?
        return _ParsedFunc(full, "", consumed)
      end
    end

    // Otherwise: split "name args" on space or comma (legacy
    // wildcard/shell function-call shape). Plain `$(VAR)` falls
    // through to the no-split case.
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

  fun _call(name: String, args: String, auto: (AutoVars | None),
    vars: Map[String, String] box, auth: FileAuth,
    expanding: Set[String]): String
  =>
    match name
    | "wildcard" => Wildcard.expand(auth, Strs.trim(args))
    | "shell" => Shell.capture(Strs.trim(args))
    else
      // Substitution reference: `$(VAR:s1=s2)` — replace s1 suffix with s2
      // in each whitespace-separated word of VAR's value.
      try
        let colon_idx = name.find(":")?
        let pattern: String val = name.substring(colon_idx + 1)
        let eq_idx = pattern.find("=")?
        // Trim the var name in case the source had whitespace like
        // `$(VAR :s1=s2)` — _parse_func tolerates it; trim here makes
        // the lookup match a normal identifier.
        let var_name: String val = Strs.trim(name.substring(0, colon_idx))
        let s1_raw: String val = pattern.substring(0, eq_idx)
        let s2_raw: String val = pattern.substring(eq_idx + 1)
        return _substitute(var_name, s1_raw, s2_raw,
          auto, vars, auth, expanding)
      end
      // Plain variable lookup.
      if expanding.contains(name) then
        ""
      else
        let raw = try vars(name)? else "" end
        expanding.set(name)
        let result = _expand(raw, auto, vars, auth, expanding)
        expanding.unset(name)
        result
      end
    end

  fun _substitute(var_name: String, s1_raw: String, s2_raw: String,
    auto: (AutoVars | None), vars: Map[String, String] box,
    auth: FileAuth, expanding: Set[String]): String
  =>
    let value =
      if expanding.contains(var_name) then
        ""
      else
        let raw = try vars(var_name)? else "" end
        expanding.set(var_name)
        let r = _expand(raw, auto, vars, auth, expanding)
        expanding.unset(var_name)
        r
      end
    let s1 = _expand(s1_raw, auto, vars, auth, expanding)
    let s2 = _expand(s2_raw, auto, vars, auth, expanding)
    let words = Strs.split_ws(value)
    let result = Array[String]
    for w in words.values() do
      if s1.size() == 0 then
        result.push(w + s2)
      elseif (s1.size() <= w.size())
          and (w.substring((w.size() - s1.size()).isize()) == s1)
      then
        result.push(
          w.substring(0, (w.size() - s1.size()).isize()) + s2)
      else
        result.push(w)
      end
    end
    " ".join(result.values())
