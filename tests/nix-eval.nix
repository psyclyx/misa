let
  npins = import ../npins;
  pkgs = import npins.nixpkgs { };
  project = import ../default.nix { inherit pkgs; };
  allTreeSitterGrammars = pkgs.tree-sitter.withPlugins (_: pkgs.tree-sitter-grammars.allGrammars);
  inherit (project) lib;
  standard = lib.standardExtensions;
  custom = ../extensions/agent.fnl;
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
    actions = "actions";
    clipboard = "clipboard";
    editing = "editing";
    costs = "costs";
    history = "history";
    queue = "queue";
    queueView = "queue_view";
    images = "images";
    attachments = "attachments";
    componentImage = "component.image";

    selection = "selection";
    selectionDocument = "selection_document";
    componentSelection = "component.selection";
    agent = "agent";
    animations = "animations";
    animationDefault = "animation.default";
    auth = "auth";
    choices = "choices";
    choiceLayout = "choice_layout";
    commands = "commands";
    omnipicker = "omnipicker";
    dialogs = "dialogs";
    dialogView = "dialog_view";
    components = "components";
    layout = "layout";
    markdown = "markdown";
    componentMarkdown = "component.markdown";
    componentTool = "component.tool";
    componentMessage = "component.message";
    componentEditor = "component.editor";
    componentPicker = "component.picker";
    componentStatus = "component.status";
    componentChrome = "component.chrome";
    componentDialog = "component.dialog";
    editor = "editor";
    fuzzy = "fuzzy";
    keybindings = "keybindings";
    indicators = "indicators";
    json = "json";
    messages = "messages";
    models = "models";
    picker = "picker";
    pickerView = "picker_view";
    preferences = "preferences";
    requestOptions = "request_options";
    effort = "effort";
    status = "status";
    themes = "themes";
    themeDefault = "theme.default";
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
assert pkgs.lib.hasSuffix "-agent.fnl" serializedCustom;
assert builtins.getContext serializedCustom != { };
assert invalid.success == false;
assert
  moduleEval.config.programs.misa.extensions == [
    "agent"
    custom
    "provider.fake"
  ];
assert builtins.pathExists ../extensions/agent.fnl;
assert builtins.pathExists ../extensions/auth.fnl;
assert builtins.pathExists ../extensions/actions.fnl;
assert builtins.pathExists ../extensions/clipboard.fnl;
assert builtins.pathExists ../extensions/editing.fnl;
assert builtins.pathExists ../extensions/selection.fnl;
assert builtins.pathExists ../extensions/selection_document.fnl;
assert builtins.pathExists ../extensions/component/selection.fnl;

assert builtins.pathExists ../extensions/animations.fnl;
assert builtins.pathExists ../extensions/animation/default.fnl;
assert builtins.pathExists ../extensions/components.fnl;
assert builtins.pathExists ../extensions/layout.fnl;
assert builtins.pathExists ../extensions/markdown.fnl;
assert builtins.pathExists ../extensions/component/markdown.fnl;
assert builtins.pathExists ../extensions/component/tool.fnl;
assert builtins.pathExists ../extensions/component/message.fnl;
assert builtins.pathExists ../extensions/component/editor.fnl;
assert builtins.pathExists ../extensions/component/picker.fnl;
assert builtins.pathExists ../extensions/component/status.fnl;
assert builtins.pathExists ../extensions/component/chrome.fnl;
assert builtins.pathExists ../extensions/editor.fnl;
assert builtins.pathExists ../extensions/choices.fnl;
assert builtins.pathExists ../extensions/choice_layout.fnl;
assert builtins.pathExists ../extensions/commands.fnl;
assert builtins.pathExists ../extensions/omnipicker.fnl;
assert builtins.pathExists ../extensions/fuzzy.fnl;
assert builtins.pathExists ../extensions/keybindings.fnl;
assert builtins.pathExists ../extensions/indicators.fnl;
assert builtins.pathExists ../extensions/json.fnl;
assert builtins.pathExists ../extensions/messages.fnl;
assert builtins.pathExists ../extensions/models.fnl;
assert builtins.pathExists ../extensions/picker.fnl;
assert builtins.pathExists ../extensions/picker_view.fnl;
assert builtins.pathExists ../extensions/preferences.fnl;
assert builtins.pathExists ../extensions/request_options.fnl;
assert builtins.pathExists ../extensions/effort.fnl;
assert builtins.pathExists ../extensions/status.fnl;
assert builtins.pathExists ../extensions/themes.fnl;
assert builtins.pathExists ../extensions/theme/default.fnl;
assert builtins.pathExists ../extensions/provider/fake.fnl;
assert builtins.pathExists ../extensions/provider/command.fnl;
assert builtins.pathExists ../extensions/provider/claude.fnl;
assert builtins.pathExists ../extensions/protocol/anthropic.fnl;
assert builtins.pathExists ../extensions/provider/anthropic.fnl;
assert builtins.pathExists ../extensions/provider/kimi.fnl;
assert builtins.pathExists ../extensions/protocol/openai.fnl;
assert builtins.pathExists ../extensions/provider/openai.fnl;
assert builtins.pathExists ../extensions/provider/openai-codex.fnl;
assert builtins.pathExists ../extensions/provider/openrouter.fnl;
assert builtins.pathExists ../extensions/tool/files.fnl;
assert builtins.pathExists ../extensions/tool/shell.fnl;
assert builtins.pathExists ../extensions/ui.fnl;
# Instantiate both the package and configured wrapper without recursively
# building either from this evaluation-only test.
assert pkgs.lib.hasSuffix ".drv" project.packages.misa.drvPath;
assert project.packages.misa.treeSitterGrammars.outPath == allTreeSitterGrammars.outPath;
assert pkgs.lib.hasSuffix ".drv" configured.drvPath;
assert configured.unwrapped.outPath == project.packages.misa.outPath;
true
