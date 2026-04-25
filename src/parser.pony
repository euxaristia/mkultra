use "collections"
use "files"

primitive Parser
  fun parse(content: String, dag: Dag, auth: FileAuth): (None | String) =>
    """
    Parse a makefile into the given Dag. Returns None on success, or an
    error message string.
    """
    var cur_tgt: String = ""
    let phony_targets = Array[String]
    var pending_recipe: (String | None) = None

    let lines: Array[String] ref = content.split("\n")
    var li: USize = 0
    let nlines = lines.size()

    try
      while li < nlines do
        let raw_line = lines(li)?

        // Recipe lines start with a tab
        if (raw_line.size() > 0) and (raw_line(0)? == '\t') then
          let trimmed: String val = Strs.trim(raw_line)
          if (trimmed.size() > 0) and (cur_tgt.size() > 0) then
            // Backslash continuation: trim ends with '\'
            let ends_bs = try trimmed(trimmed.size() - 1)? == '\\' else false end
            if ends_bs then
              let cmd: String val = trimmed.substring(0, (trimmed.size() - 1).isize())
              let prev: String val = match pending_recipe
                | let p: String => p
                | None => ""
              end
              pending_recipe = prev + cmd + "\n"
            else
              let prev: String val = match pending_recipe
                | let p: String => p
                | None => ""
              end
              dag.add_recipe(cur_tgt, prev + trimmed)
              pending_recipe = None
            end
          end
          li = li + 1
          continue
        end

        // Flush pending recipe before processing a non-recipe line
        match pending_recipe
        | let cmd: String =>
          let t = Strs.trim(cmd)
          if (cur_tgt.size() > 0) and (t.size() > 0) then
            dag.add_recipe(cur_tgt, t)
          end
          pending_recipe = None
        end

        let trimmed = Strs.trim(raw_line)
        if trimmed.size() == 0 then
          li = li + 1
          continue
        end
        if trimmed(0)? == '#' then
          li = li + 1
          continue
        end

        // Try `:=` (simply-expanded variable assignment)
        var handled = false
        try
          let pos = trimmed.find(":=")?.usize()
          let lhs_raw: String val = trimmed.substring(0, pos.isize())
          let rhs_raw: String val = trimmed.substring((pos + 2).isize())
          let lhs: String val = Strs.trim(lhs_raw)
          let rhs: String val = Strs.trim(rhs_raw)
          if (lhs.size() > 0) and (not lhs.contains(" ")) then
            let expanded: String val = Expand.simple(rhs, dag.variables, auth)
            dag.set_variable(lhs, expanded)
            cur_tgt = ""
            handled = true
          end
        end
        if handled then
          li = li + 1
          continue
        end

        // Try `=`, `?=`, `+=`
        try
          let eq: USize = trimmed.find("=")?.usize()
          let before_eq: String val = trimmed.substring(0, eq.isize())
          let bef: String val = Strs.trim(before_eq)
          // If the trimmed lhs ends in ':', this is `:=` syntax — already
          // handled above. (This branch is reached when the := pre-pass
          // bailed for some reason.)
          if (bef.size() > 0) and (try bef(bef.size() - 1)? == ':' else false end) then
            error
          end

          let char_before: U8 =
            if eq > 0 then
              try trimmed(eq - 1)? else 0 end
            else
              0
            end
          let is_qp = (char_before == '?') or (char_before == '+')
          let lhs_for_store: String val =
            if is_qp then
              Strs.trim(bef.substring(0, (bef.size() - 1).isize()))
            else
              bef
            end
          if (lhs_for_store.size() == 0)
              or lhs_for_store.contains(" ")
              or lhs_for_store.contains(":")
          then
            error
          end
          let rhs_raw: String val = trimmed.substring((eq + 1).isize())
          let rhs: String val = Strs.trim(rhs_raw)
          if char_before == '?' then
            if not dag.variables.contains(lhs_for_store) then
              dag.set_variable(lhs_for_store, rhs)
            end
          elseif char_before == '+' then
            let cur = try dag.variables(lhs_for_store)? else "" end
            let sep = if cur.size() == 0 then "" else " " end
            dag.set_variable(lhs_for_store, cur + sep + rhs)
          else
            dag.set_variable(lhs_for_store, rhs)
          end
          cur_tgt = ""
          handled = true
        end
        if handled then
          li = li + 1
          continue
        end

        // Rule line: `target: prereqs`
        try
          let colon: USize = trimmed.find(":")?.usize()
          let after_colon: String val = trimmed.substring((colon + 1).isize())
          // Skip `:=` (shouldn't reach here normally — variable branch handled it)
          if (after_colon.size() > 0) and (try after_colon(0)? == '=' else false end) then
            cur_tgt = ""
            error
          end
          let target_part_raw: String val = trimmed.substring(0, colon.isize())
          let target_part: String val = Strs.trim(target_part_raw)
          let expanded_target: String val = Expand.simple(target_part, dag.variables, auth)
          let trimmed_prereqs: String val = Strs.trim(after_colon)
          let expanded_prereqs: String val = Expand.simple(trimmed_prereqs, dag.variables, auth)
          cur_tgt = expanded_target

          let is_phony = expanded_target == ".PHONY"
          dag.ensure_node(expanded_target, is_phony)

          if expanded_target == ".PHONY" then
            for name in Strs.split_ws(expanded_prereqs).values() do
              phony_targets.push(name)
              dag.ensure_node(name, true)
            end
          elseif (expanded_target != ".SUFFIXES")
              and (try expanded_target(0)? != '.' else true end)
              and (dag.default_target is None)
          then
            dag.set_default(expanded_target)
          end

          if (expanded_prereqs.size() > 0) and (expanded_target != ".PHONY") then
            for prereq in Strs.split_ws(expanded_prereqs).values() do
              dag.add_prereq(expanded_target, prereq)
            end
          end
        end
        li = li + 1
      end
    end

    // Flush pending recipe at end of file
    match pending_recipe
    | let cmd: String =>
      let t = Strs.trim(cmd)
      if (cur_tgt.size() > 0) and (t.size() > 0) then
        dag.add_recipe(cur_tgt, t)
      end
    end

    // Mark phony targets
    for name in phony_targets.values() do
      try dag.nodes(name)?.is_phony = true end
    end

    None
