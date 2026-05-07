-- file_tree.lua — interactive file-tree plugin for devix.
--
-- Renders a collapsible directory tree into the left sidebar. Click a
-- directory row to toggle its expansion (re-listing children with
-- `devix.read_dir`); press Enter on a file row to open it through
-- `devix.open_path`. Up/Down move the selection; the selected row is
-- prefixed with `>` so it's visible against the regular row markers.

-- ─────────────────────────────────────────────────────────────────────
-- State
-- ─────────────────────────────────────────────────────────────────────

local cwd = devix.cwd()

-- Tree of expanded paths plus their cached child listings. Keyed by
-- absolute path; presence in the table means "expanded". The value is
-- a sorted list of { name, is_dir, path }.
local expanded = {}

-- Flat list of visible rows, rebuilt every time we re-render. Each
-- entry: { depth, name, is_dir, path }. The header row (cwd) sits at
-- index 1; a blank spacer at index 2 keeps the file rows visually
-- separated. Selection indexes into this list with a 1-based row id
-- pointing at content rows only (header/spacer skipped on movement).
local rows = {}
local selected_row = 3 -- first content row

local function path_join(parent, name)
    if parent:sub(-1) == "/" then
        return parent .. name
    end
    return parent .. "/" .. name
end

local function list_children(path)
    -- Cache lookup: list_children only re-reads when the dir is
    -- expanded for the first time (or after collapse/expand). Errors
    -- (permission denied, missing dir) bubble up via pcall so the
    -- plugin keeps running.
    local ok, entries = pcall(devix.read_dir, path)
    if not ok or type(entries) ~= "table" then
        return {}
    end
    local dirs, files = {}, {}
    for _, e in ipairs(entries) do
        if e.name:sub(1, 1) ~= "." then
            local row = { name = e.name, is_dir = e.is_dir, path = path_join(path, e.name) }
            if e.is_dir then table.insert(dirs, row) else table.insert(files, row) end
        end
    end
    table.sort(dirs, function(a, b) return a.name < b.name end)
    table.sort(files, function(a, b) return a.name < b.name end)
    local out = {}
    for _, r in ipairs(dirs) do table.insert(out, r) end
    for _, r in ipairs(files) do table.insert(out, r) end
    return out
end

local function append_subtree(out, dir_path, depth)
    -- DFS expansion: emit each child, recursing into expanded dirs.
    local kids = list_children(dir_path)
    for _, k in ipairs(kids) do
        table.insert(out, { depth = depth, name = k.name, is_dir = k.is_dir, path = k.path })
        if k.is_dir and expanded[k.path] then
            append_subtree(out, k.path, depth + 1)
        end
    end
end

local function rebuild_rows()
    rows = {}
    -- Header + spacer.
    table.insert(rows, { depth = 0, name = cwd, is_dir = true, path = cwd, header = true })
    table.insert(rows, { spacer = true })
    append_subtree(rows, cwd, 0)
    -- Clamp selection. Content starts at index 3.
    local first_content = 3
    local last_content = #rows
    if selected_row < first_content then selected_row = first_content end
    if selected_row > last_content then selected_row = last_content end
end

-- ─────────────────────────────────────────────────────────────────────
-- Render
-- ─────────────────────────────────────────────────────────────────────

local function format_row(idx, row)
    if row.header then return row.name end
    if row.spacer then return "" end
    local indent = string.rep("  ", row.depth)
    local marker
    if row.is_dir then
        marker = expanded[row.path] and "▾ " or "▸ "
    else
        marker = "  "
    end
    local prefix = (idx == selected_row) and ">" or " "
    return prefix .. indent .. marker .. row.name
end

local function ensure_selection_visible(pane)
    -- Adjust scroll so `selected_row` stays inside the painted body
    -- height. visible_rows is 0 before the first paint; on that first
    -- repaint we just leave scroll alone — the second paint corrects
    -- it once the renderer has reported a real height.
    local visible = pane:visible_rows()
    if visible == 0 then return end
    local top = pane:scroll()
    -- rows are 1-indexed in Lua but the renderer's scroll is 0-based.
    local sel_idx = selected_row - 1
    if sel_idx < top then
        pane:scroll_to(sel_idx)
    elseif sel_idx >= top + visible then
        pane:scroll_to(sel_idx - visible + 1)
    end
end

local function repaint(pane)
    rebuild_rows()
    local lines = {}
    for i, r in ipairs(rows) do
        table.insert(lines, format_row(i, r))
    end
    pane:set_lines(lines)
    ensure_selection_visible(pane)
end

-- ─────────────────────────────────────────────────────────────────────
-- Pane registration
-- ─────────────────────────────────────────────────────────────────────

local pane = devix.register_pane({ slot = "left" })

local function activate(idx)
    local row = rows[idx]
    if not row or row.header or row.spacer then return end
    if row.is_dir then
        expanded[row.path] = not expanded[row.path] and true or nil
    else
        devix.open_path(row.path)
    end
    -- Repaint regardless of branch so the `>` selector marker reflects
    -- the new selected row even when the row is a file (no expansion
    -- change).
    repaint(pane)
end

local function move_selection(delta)
    local first_content = 3
    local last_content = #rows
    if last_content < first_content then return end
    local target = selected_row + delta
    if target < first_content then target = first_content end
    if target > last_content then target = last_content end
    selected_row = target
    repaint(pane)
end

pane:on_key(function(ev)
    if ev.key == "down" or (ev.key == "j" and not ev.ctrl and not ev.alt) then
        move_selection(1)
    elseif ev.key == "up" or (ev.key == "k" and not ev.ctrl and not ev.alt) then
        move_selection(-1)
    elseif ev.key == "enter" then
        activate(selected_row)
    end
end)

pane:on_click(function(ev)
    -- Click y is pane-relative; add the current scroll offset to map
    -- it back to a row index in the (1-based) `rows` list.
    local idx = ev.y + pane:scroll() + 1
    if idx >= 1 and idx <= #rows then
        selected_row = idx
        activate(idx)
    end
end)

repaint(pane)

devix.register_action({
    id = "filetree.refresh",
    label = "File Tree: Refresh",
    chord = "ctrl+e",
    run = function()
        -- Drop cached child lists so the next paint re-reads disk.
        expanded = {}
        repaint(pane)
        devix.status("file tree refreshed at " .. cwd)
    end,
})
