-- Semantic theme data and style composition. Components name roles; this service
-- alone resolves those names to the closed native style record.
return {
  setup = function(context)
    local entries, registrations_sealed = {}, false
    local config = type(context.config) == "table" and context.config.themes or nil
    config = type(config) == "table" and config or {}
    local configured = type(config.default) == "string" and config.default or "default"
    local ansi = {black=true,red=true,green=true,yellow=true,blue=true,magenta=true,cyan=true,white=true,
      bright_black=true,bright_red=true,bright_green=true,bright_yellow=true,bright_blue=true,bright_magenta=true,bright_cyan=true,bright_white=true}
    local attributes = {bold=true,italic=true,dim=true,strikethrough=true,underline=true}

    local function rgb(value)
      if type(value) ~= "table" then return nil end
      local count=0; for key in pairs(value) do assert(key=="r" or key=="g" or key=="b","unknown RGB field: "..tostring(key)); count=count+1 end
      assert(count==3,"RGB color requires r, g, and b")
      local result={}; for _,key in ipairs({"r","g","b"}) do local channel=value[key]
        assert(type(channel)=="number" and channel%1==0 and channel>=0 and channel<=255,"RGB channels must be bytes"); result[key]=channel
      end
      return result
    end
    local function terminal_color(value)
      if type(value)=="string" then assert(value=="default" or ansi[value],"unknown terminal color: "..value); return value end
      return assert(rgb(value),"foreground must be an ANSI color, default, or RGB record")
    end
    local function copy_color(value)
      return type(value)=="table" and {r=value.r,g=value.g,b=value.b} or value
    end
    local function normalize_style(value,palette,name)
      assert(type(value)=="table","theme style must be a record: "..name)
      local result={}
      for key,field in pairs(value) do
        if key=="foreground" then
          if type(field)=="string" and palette[field]~=nil then result.foreground=copy_color(palette[field]) else result.foreground=terminal_color(field) end
        else
          assert(attributes[key],"unknown style field: "..tostring(key)); assert(type(field)=="boolean","style attribute must be boolean: "..key); result[key]=field
        end
      end
      assert(next(result)~=nil,"theme style must not be empty: "..name)
      return result
    end
    local function normalize_theme(theme)
      assert(type(theme.palette)=="table" and type(theme.styles)=="table","theme requires palette and styles records")
      local palette={}; for name,color in pairs(theme.palette) do
        assert(type(name)=="string" and name~="","palette names must be nonempty strings"); palette[name]=terminal_color(color)
      end
      local styles={}; for name,style in pairs(theme.styles) do
        assert(type(name)=="string" and name~="","style token must be nonempty"); styles[name]=normalize_style(style,palette,name)
      end
      for _,required in ipairs({"plain","label","value","keybinding"}) do assert(styles[required],"theme is missing standard token: "..required) end
      return {palette=palette,styles=styles}
    end
    local function merge(target,source)
      for key,value in pairs(source) do target[key]=copy_color(value) end
    end

    misa.reg_theme = function(id, theme)
      assert(not registrations_sealed, "theme registrations are sealed")
      assert(type(id) == "string" and id ~= "" and type(theme) == "table", "invalid theme")
      assert(entries[id] == nil, "duplicate theme: " .. id)
      entries[id] = normalize_theme(theme)
    end
    misa.theme = function(db)
      assert(type(db) == "table" and type(db.themes) == "table", "theme resolution requires initialized db")
      return assert(entries[db.themes.active], "unknown theme: " .. tostring(db.themes.active))
    end
    misa.theme_style = function(db, tokens)
      if type(tokens)=="string" then tokens={tokens} end
      assert(type(tokens)=="table" and #tokens>0,"span style requires one or more semantic tokens")
      local styles,result=misa.theme(db).styles,{}
      for _,name in ipairs(tokens) do
        assert(type(name)=="string" and name~="","semantic style tokens must be nonempty strings")
        merge(result,assert(styles[name],"theme has no style token: "..name))
      end
      return result
    end
    misa.swap_theme = function(db, id)
      assert(entries[id], "unknown theme: " .. tostring(id))
      db.themes.active = id
    end
    misa.reg_interceptor({ id = "themes/initialize", before = function(tx)
      if tx.event.type == "app/start" then
        registrations_sealed = true
        if not tx.db.themes then tx.db.themes = { active = configured } end
      end
      return tx
    end })
    misa.reg_event("app/start", function(db)
      if config.persist == false then return { db = db } end
      return { db = db, fx = { { type = "state/load", namespace = "ui.theme", completion = "themes/loaded" } } }
    end)
    misa.reg_event("themes/loaded", function(db, event)
      if event.found == false or event.data == misa.json_null then return { db = db } end
      assert(type(event.data) == "table" and type(event.data.active) == "string", "invalid persisted theme")
      if entries[event.data.active] then db.themes.active = event.data.active end
      return { db = db }
    end)
    misa.reg_event("themes/swap", function(db, event)
      misa.swap_theme(db, event.theme)
      local fx = {}
      if config.persist ~= false then fx[#fx + 1] = { type = "state/save", namespace = "ui.theme", data = db.themes } end
      fx[#fx + 1] = { type = "dispatch", event = { type = "ui/redraw" } }
      return { db = db, fx = fx }
    end)
  end,
}
