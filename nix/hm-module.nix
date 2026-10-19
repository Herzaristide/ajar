# Module Home Manager. Dans /etc/nixos :
#   inputs.ajar.url = "path:/home/aristide/ciph3r/ajar";
#   imports = [ inputs.ajar.homeManagerModules.default ];
#   programs.ajar.enable = true;
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.ajar;
  toml = pkgs.formats.toml { };
in
{
  options.programs.ajar = {
    enable = lib.mkEnableOption "Ajar, l'espace d'écriture zen";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "Le paquet Ajar.";
    };

    fonts = lib.mkOption {
      type = lib.types.listOf lib.types.package;
      default = with pkgs; [ literata eb-garamond ];
      description = "Polices installées pour le texte.";
    };

    settings = lib.mkOption {
      inherit (toml) type;
      default = { };
      example = {
        police = "EB Garamond";
        taille = 20;
        largeur = 65;
        theme = "papier";
        typographie = true;
        plein_ecran = true;
      };
      description = "Écrit dans ~/.config/ajar/config.toml.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ] ++ cfg.fonts;
    fonts.fontconfig.enable = lib.mkIf (cfg.fonts != [ ]) (lib.mkDefault true);
    xdg.configFile."ajar/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "ajar-config.toml" cfg.settings;
    };
  };
}
