use "collections"

class DagNode
  let target: String
  var prereqs: Array[String] = Array[String]
  var recipes: Array[String] = Array[String]
  var is_phony: Bool

  new create(target': String, is_phony': Bool) =>
    target = target'
    is_phony = is_phony'

class Dag
  let nodes: Map[String, DagNode] = Map[String, DagNode]
  let variables: Map[String, String] = Map[String, String]
  let _overridden: Set[String] = Set[String]
  var default_target: (String | None) = None

  fun ref set_variable(name: String, value: String) =>
    if not _overridden.contains(name) then
      variables(name) = value
    end

  fun ref set_override(name: String, value: String) =>
    variables(name) = value
    _overridden.set(name)

  fun ref ensure_node(target: String, is_phony: Bool): DagNode =>
    try
      nodes(target)?
    else
      let nd = DagNode(target, is_phony)
      nodes(target) = nd
      nd
    end

  fun ref add_prereq(target: String, prereq: String) =>
    ensure_node(target, false).prereqs.push(prereq)
    ensure_node(prereq, false)
    if (default_target is None)
        and (try target(0)? != '.' else true end)
        and (target != ".PHONY")
        and (target != ".SUFFIXES")
    then
      set_default(target)
    end

  fun ref add_recipe(target: String, recipe: String) =>
    ensure_node(target, false).recipes.push(recipe)

  fun ref set_default(target: String) =>
    if default_target is None then
      default_target = target
    end

  fun box detect_cycle(): (Array[String] | None) =>
    """
    Iterative DFS with three-coloring. Returns the cycle path (in order)
    if a cycle is found, else None.
    """
    let color = Map[String, U8]
    let parent = Map[String, String]
    for k in nodes.keys() do color(k) = 0 end

    let starts = Array[String]
    for k in nodes.keys() do starts.push(k) end

    for start in starts.values() do
      if (try color(start)? else 2 end) != 0 then continue end

      // stack of (node, next_prereq_index)
      let stack = Array[(String, USize)]
      stack.push((start, 0))

      try
        while stack.size() > 0 do
          (let node, let idx) = stack(stack.size() - 1)?
          let c = try color(node)? else 0 end
          if c == 2 then
            stack.pop()?
            continue
          end
          if c == 0 then color(node) = 1 end

          let prs =
            try nodes(node)?.prereqs else Array[String] end

          var pushed = false
          var next_idx = idx
          while next_idx < prs.size() do
            let prereq = prs(next_idx)?
            let pc = try color(prereq)? else 0 end
            if pc == 0 then
              parent(prereq) = node
              color(prereq) = 0
              stack(stack.size() - 1)? = (node, next_idx + 1)
              stack.push((prereq, 0))
              pushed = true
              break
            elseif pc == 1 then
              let cycle = Array[String]
              cycle.push(prereq)
              cycle.push(node)
              var cur = node
              var hops: USize = 0
              let max_hops = nodes.size()
              while hops < max_hops do
                let p = try parent(cur)? else break end
                if p == prereq then break end
                cycle.push(p)
                cur = p
                hops = hops + 1
              end
              cycle.reverse_in_place()
              return cycle
            end
            next_idx = next_idx + 1
          end

          if not pushed then
            color(node) = 2
            stack.pop()?
          end
        end
      end
    end
    None

  fun ref order(target: String): Array[DagNode] =>
    """
    Topological sort starting from `target`. Returns build order.
    """
    let visited = Set[String]
    let out = Array[String]
    _topo(target, visited, out)
    let result = Array[DagNode]
    for name in out.values() do
      try result.push(nodes(name)?) end
    end
    result

  fun box _topo(name: String, visited: Set[String], out: Array[String]) =>
    if visited.contains(name) then return end
    visited.set(name)
    try
      for prereq in nodes(name)?.prereqs.values() do
        _topo(prereq, visited, out)
      end
    end
    out.push(name)
