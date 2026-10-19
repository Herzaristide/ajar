# Module Home Manager. Dans /etc/nixos :
#   inputs.scriptorium.url = "path:/home/aristide/ciph3r/scriptorium";
#   imports = [ inputs.scriptorium.homeManagerModules.default ];
#   programs.scriptorium.enable = true;
self:
{ config, lib, pkgs, ... }:

let
  cfg = config.programs.scriptorium;
  toml = pkgs.formats.toml { };
in
{
  options.programs.scriptorium = {
    enable = lib.mkEnableOption "Scriptorium, l'espace d'écriture zen";

    package = lib.mkOption {
      type = lib.types.package;
      default = self.packages.${pkgs.stdenv.hostPlatform.system}.default;
      description = "Le paquet Scriptorium.";
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
      description = "Écrit dans ~/.config/scriptorium/config.toml.";
    };
  };

  config = lib.mkIf cfg.enable {
    home.packages = [ cfg.package ] ++ cfg.fonts;
    fonts.fontconfig.enable = lib.mkIf (cfg.fonts != [ ]) (lib.mkDefault true);
    xdg.configFile."scriptorium/config.toml" = lib.mkIf (cfg.settings != { }) {
      source = toml.generate "scriptorium-config.toml" cfg.settings;
    };
  };
}
