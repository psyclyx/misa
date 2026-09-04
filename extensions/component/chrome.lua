-- Static application chrome; dynamic model/session facts belong in status.
return {setup=function()
  misa.reg_component("default.root.header",{render=function() return {lines={
    {spans={{text="misa",style="bold"},{text="  coding agent",style="dim"}}},
    {spans={{text="interactive session",style="dim"}}},
  }} end})
end}
