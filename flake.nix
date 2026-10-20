{
  description = "Thornhill: Ambient Cognitive Backbone for Enyxma OS";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
      in
      {
        devShells.default = pkgs.mkShell {
          name = "thornhill-dev";
          nativeBuildInputs = with pkgs; [
            cargo
            rustc
            rustfmt
            clippy
            pkg-config
            just
            sqlite
          ];
          buildInputs = with pkgs; [
            sqlite
          ];
          shellHook = ''
            echo "👁️ Thornhill Bilişsel Çekirdek Geliştirme Ortamı Hazır!"
            echo "Anayasa: docs/rfc-001.md | Doğrulama: just verify"
          '';
        };

        packages.default = pkgs.stdenv.mkDerivation {
          pname = "thornhill";
          version = "0.1.0-rfc001";
          src = ./.;
          dontBuild = true;
          installPhase = ''
            mkdir -p $out/share/thornhill/docs $out/bin
            cp docs/rfc-001.md $out/share/thornhill/docs/
          '';
          meta = with pkgs.lib; {
            description = "Ambient cognitive backbone: sensory tap, causal graph, dual-mind reflex engine";
            license = licenses.mit;
            platforms = platforms.linux;
          };
        };
      }) // {
        # Evrensel NixOS Modülü
        nixosModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.thornhill;
          in
          {
            options.services.thornhill = {
              enable = lib.mkEnableOption "Enyxma Thornhill Cognitive Daemon";
            };
          };
      };
}
