let
  npins = import ../npins;
  pkgs = import npins.nixpkgs { };
  project = import ../default.nix { inherit pkgs; };
  inherit (project) lib;
  standard = lib.standardExtensions;
  custom = ../extensions/agent.lua;
  defaults = lib.mkMisa { };
  configured = lib.mkMisa {
    extensions = with standard; [
      agent
      custom
      providerCommand
    ];
  };
  serializedCustom = builtins.elemAt configured.configData.extensions 1;
  invalid = builtins.tryEval (
    builtins.deepSeq (lib.mkMisa { extensions = [ "provider.unknown" ]; }).configData true
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
        programs.misa.extensions = [
          "agent"
          custom
          "provider.fake"
        ];
      }
    ];
  };
in
assert
  standard == {
    agent = "agent";
    models = "models";
    providerFake = "provider.fake";
    providerCommand = "provider.command";
    providerClaude = "provider.claude";
    protocolAnthropic = "protocol.anthropic";
    providerAnthropic = "provider.anthropic";
    providerKimi = "provider.kimi";
    protocolOpenAI = "protocol.openai";
    providerOpenAI = "provider.openai";
    providerOpenAICodex = "provider.openai-codex";
    providerOpenRouter = "provider.openrouter";
    toolFiles = "tool.files";
    toolShell = "tool.shell";
    ui = "ui";
  };
assert
  defaults.configData == {
    extensions = [ ];
    config = { };
  };
assert builtins.elemAt configured.configData.extensions 0 == "agent";
assert builtins.elemAt configured.configData.extensions 2 == "provider.command";
# Do not pin the content hash: verify path interpolation performed store
# coercion and retained dependency context for writeText's closure.
assert pkgs.lib.hasPrefix "${builtins.storeDir}/" serializedCustom;
assert pkgs.lib.hasSuffix "-agent.lua" serializedCustom;
assert builtins.getContext serializedCustom != { };
assert invalid.success == false;
assert
  moduleEval.config.programs.misa.extensions == [
    "agent"
    custom
    "provider.fake"
  ];
assert builtins.pathExists ../extensions/agent.lua;
assert builtins.pathExists ../extensions/models.lua;
assert builtins.pathExists ../extensions/provider/fake.lua;
assert builtins.pathExists ../extensions/provider/command.lua;
assert builtins.pathExists ../extensions/provider/claude.lua;
assert builtins.pathExists ../extensions/protocol/anthropic.lua;
assert builtins.pathExists ../extensions/provider/anthropic.lua;
assert builtins.pathExists ../extensions/provider/kimi.lua;
assert builtins.pathExists ../extensions/protocol/openai.lua;
assert builtins.pathExists ../extensions/provider/openai.lua;
assert builtins.pathExists ../extensions/provider/openai-codex.lua;
assert builtins.pathExists ../extensions/provider/openrouter.lua;
assert builtins.pathExists ../extensions/tool/files.lua;
assert builtins.pathExists ../extensions/tool/shell.lua;
assert builtins.pathExists ../extensions/ui.lua;
# Instantiate both the package and configured wrapper without recursively
# building either from this evaluation-only test.
assert pkgs.lib.hasSuffix ".drv" project.packages.misa.drvPath;
assert pkgs.lib.hasSuffix ".drv" configured.drvPath;
assert configured.unwrapped.outPath == project.packages.misa.outPath;
true
