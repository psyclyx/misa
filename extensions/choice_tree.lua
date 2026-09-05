-- Tree projection for generic choices. Paths are semantic IDs, never display labels.
local function delimiter_at(value,start) return value:find("[/.:]",start) end
local function parent(node)
  if node=="" then return "" end
  local trimmed=node:sub(1,-2); local last
  for at in trimmed:gmatch("()[/.:]") do last=at end
  return last and trimmed:sub(1,last) or ""
end
local function searchable(item)
  local fields={item.id,item.path or "",item.label or "",item.description or ""}
  if type(item.search)=="string" then fields[#fields+1]=item.search elseif type(item.search)=="table" then for _,v in ipairs(item.search) do fields[#fields+1]=v end end
  return table.concat(fields," ")
end
return {setup=function()
  misa.choice_tree_parent=parent
  misa.choice_tree_project=function(session,match)
    local node=session.tree.node; local candidates=session.query=="" and session.items or match(session.items,session.query)
    local by_id,order={},{}
    for _,item in ipairs(candidates) do
      local path=item.path or item.id
      if path:sub(1,#node)==node then
        local rest=path:sub(#node+1); local delimiter=delimiter_at(rest,1)
        if delimiter==1 then delimiter=delimiter_at(rest,2) end
        if delimiter and delimiter<#rest then
          local prefix=node..rest:sub(1,delimiter)
          if not by_id[prefix] then
            by_id[prefix]={id="tree:"..prefix,value=prefix,path=prefix,label=rest:sub(1,delimiter),search="",tree_prefix=prefix}
            order[#order+1]=prefix
          end
          by_id[prefix].search=by_id[prefix].search.." "..searchable(item)
        else
          local leaf={}; for key,value in pairs(item) do leaf[key]=value end
          leaf.label=rest~="" and rest or path; leaf.description=nil
          by_id[item.id]=leaf; order[#order+1]=item.id
        end
      end
    end
    local result,seen={},{}
    for _,id in ipairs(order) do if not seen[id] then seen[id]=true; result[#result+1]=by_id[id] end end
    return result
  end
end}
