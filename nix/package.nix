{ lib
, craneLib
, pkg-config
, wrapGAppsHook4
, copyDesktopItems
, makeDesktopItem
, gtk4
, libadwaita
, literata
, eb-garamond
, writeText
}:

let
  common = {
    src = craneLib.cleanCargoSource ../.;
    strictDeps = true;
    nativeBuildInputs = [ pkg-config ];
    buildInputs = [ gtk4 libadwaita ];
  };
  cargoArtifacts = craneLib.buildDepsOnly common;

  # La configuration fontconfig du système, plus les polices du texte.
  fontsConf = writeText "ajar-fonts.conf" ''
    <?xml version="1.0"?>
    <!DOCTYPE fontconfig SYSTEM "urn:fontconfig:fonts.dtd">
    <fontconfig>
      <include ignore_missing="yes">/etc/fonts/fonts.conf</include>
      <dir>${literata}/share/fonts</dir>
      <dir>${eb-garamond}/share/fonts</dir>
    </fontconfig>
  '';
in
craneLib.buildPackage (common // {
  inherit cargoArtifacts;
  nativeBuildInputs = common.nativeBuildInputs ++ [ wrapGAppsHook4 copyDesktopItems ];

  preFixup = ''
    gappsWrapperArgs+=(--set-default FONTCONFIG_FILE ${fontsConf})
  '';

  desktopItems = [
    (makeDesktopItem {
      name = "ajar";
      desktopName = "Ajar";
      genericName = "Écriture";
      comment = "Un espace d'écriture zen pour la littérature";
      exec = "ajar %f";
      icon = "accessories-text-editor";
      categories = [ "Office" "TextEditor" ];
      startupWMClass = "ajar";
    })
  ];

  meta = {
    description = "Un espace d'écriture zen pour la littérature";
    license = lib.licenses.gpl3Plus;
    mainProgram = "ajar";
    platforms = lib.platforms.linux;
  };
})
