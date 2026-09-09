let
  # Mirror source namespaces; init selects the module owned by its directory.
  catalog =
    directory: namespace:
    let
      entries = builtins.readDir directory;
      names = builtins.filter (
        name: entries.${name} == "directory" || builtins.match ".*\\.fnl" name != null
      ) (builtins.attrNames entries);
    in
    builtins.listToAttrs (
      map (
        name:
        let
          isDirectory = entries.${name} == "directory";
          key = if isDirectory then name else builtins.substring 0 (builtins.stringLength name - 4) name;
          module = if key == "init" then namespace else namespace ++ [ key ];
        in
        {
          name = key;
          value =
            if isDirectory then
              catalog (directory + "/${name}") module
            else
              builtins.concatStringsSep "." module;
        }
      ) names
    );
in
catalog ../extensions [ ]
