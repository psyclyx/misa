let
  npins = import ../npins;
  pkgs = import npins.nixpkgs { };
  project = import ../default.nix { inherit pkgs; };
  allTreeSitterGrammars = pkgs.tree-sitter.withPlugins (_: pkgs.tree-sitter-grammars.allGrammars);
  inherit (project) lib;
  standard = lib.standardExtensions;
  checkCatalog =
    namespace: entries:
    builtins.all (
      name:
      let
        value = entries.${name};
        path = namespace ++ [ name ];
        module = if name == "init" then namespace else path;
      in
      if builtins.isAttrs value then
        checkCatalog path value
      else
        value == builtins.concatStringsSep "." module
        && builtins.pathExists (../extensions + "/${builtins.concatStringsSep "/" path}.fnl")
    ) (builtins.attrNames entries);
  custom = ../config/default.fnl;
  defaults = lib.mkMisa { };
  scripted = lib.mkMisa { configuration = custom; };
  serializedCustom = scripted.configFile;
  invalid = builtins.tryEval (
    builtins.deepSeq (lib.mkMisa { configuration = "/unpackaged/config.fnl"; }).configFile true
  );
  moduleEval = pkgs.lib.evalModules {
    specialArgs = { inherit pkgs; };
    modules = [
      {
        options.home.packages = pkgs.lib.mkOption {
          type = pkgs.lib.types.listOf pkgs.lib.types.package;
          default = [ ];
        };
      }
      project.homeManagerModules.misa
      {
        programs.misa.configuration = custom;
      }
    ];
  };
in
assert !(project.default.src.filter "${toString ../.}/.zig-cache" "directory");
assert !(project.default.src.filter "${toString ../.}/zig-out" "directory");
assert !(project.default.src.filter "${toString ../.}/.direnv" "directory");
assert project.default.src.filter "${toString ../.}/tools/compile-fennel.lua" "regular";
assert project.default.src.filter "${toString ../.}/extensions/misa/agent/init.fnl" "regular";
assert builtins.attrNames standard == [ "misa" ];
assert standard.misa.agent.init == "misa.agent";
assert standard.misa.agent.request-options == "misa.agent.request-options";
assert standard.misa.ui.components.init == "misa.ui.components";
assert standard.misa.ui.components.markdown == "misa.ui.components.markdown";
assert standard.misa.ui.themes.default == "misa.ui.themes.default";
assert standard.misa.editor.queue.view == "misa.editor.queue.view";
assert standard.misa.providers.openai-codex == "misa.providers.openai-codex";
assert standard.misa.standard == "misa.standard";
assert checkCatalog [ ] standard;
assert defaults.configFile == null;
assert pkgs.lib.hasPrefix "${builtins.storeDir}/" scripted.configFile;
assert builtins.getContext scripted.configFile != { };
assert moduleEval.config.programs.misa.configuration == custom;
# Path interpolation retains the configuration in the wrapper's closure.
assert pkgs.lib.hasSuffix "-default.fnl" serializedCustom;
assert invalid.success == false;
# Instantiate both the package and configured wrapper without recursively
# building either from this evaluation-only test.
assert pkgs.lib.hasSuffix ".drv" project.packages.misa.drvPath;
assert project.packages.misa.treeSitterGrammars.outPath == allTreeSitterGrammars.outPath;
assert pkgs.lib.hasSuffix ".drv" scripted.drvPath;
assert scripted.unwrapped.outPath == project.packages.misa.outPath;
true
