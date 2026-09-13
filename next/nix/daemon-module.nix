{
  config,
  lib,
  pkgs,
  ...
}:
let
  cfg = config.services.misa;
  arguments = [
    "--session"
    cfg.session
    "--data-dir"
    cfg.dataDir
  ]
  ++ lib.concatMap (path: [
    "--plugin"
    (toString path)
  ]) cfg.plugins
  ++ lib.optional cfg.open "--open"
  ++ lib.concatMap (id: [
    "--allow"
    id
  ]) cfg.allow
  ++ cfg.extraArgs;
in
{
  options.services.misa = {
    enable = lib.mkEnableOption "the Misa session daemon";
    package = lib.mkOption {
      type = lib.types.package;
      default = (import ../../default.nix { inherit pkgs; }).packages.misa-daemon;
      defaultText = lib.literalExpression "misa.packages.misa-daemon";
      description = "The Rust daemon package.";
    };
    session = lib.mkOption {
      type = lib.types.str;
      default = "main";
      description = "Session served by this daemon.";
    };
    dataDir = lib.mkOption {
      type = lib.types.str;
      default = "/var/lib/misa";
      readOnly = true;
      description = "Directory for the log, blobs, shell output and credentials.";
    };
    plugins = lib.mkOption {
      type = lib.types.listOf lib.types.path;
      default = [ ];
      description = "Policy components composed into the session.";
    };
    open = lib.mkOption {
      type = lib.types.bool;
      default = false;
      description = "Admit every endpoint. Otherwise admission uses pairing and the allow list.";
    };
    allow = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Endpoint public keys admitted to the session and blob protocols.";
    };
    extraArgs = lib.mkOption {
      type = lib.types.listOf lib.types.str;
      default = [ ];
      description = "Additional daemon arguments, such as provider and model selection.";
    };
  };
  config = lib.mkIf cfg.enable {
    systemd.services.misa = {
      description = "Misa session daemon";
      wantedBy = [ "multi-user.target" ];
      wants = [ "network-online.target" ];
      after = [ "network-online.target" ];
      environment.MISA_CREDENTIALS = "${cfg.dataDir}/credentials.json";
      serviceConfig = {
        ExecStart = "${cfg.package}/bin/misa-daemon ${lib.escapeShellArgs arguments}";
        DynamicUser = true;
        StateDirectory = "misa";
        StateDirectoryMode = "0700";
        WorkingDirectory = cfg.dataDir;
        ReadWritePaths = [ cfg.dataDir ];
        UMask = "0077";
        Restart = "on-failure";
      };
    };
  };
}
