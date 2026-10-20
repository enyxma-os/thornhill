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

        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "thornhill";
          version = "0.1.0";
          src = ./.;

          cargoLock = {
            lockFile = ./Cargo.lock;
          };

          nativeBuildInputs = with pkgs; [ pkg-config ];
          buildInputs = with pkgs; [ sqlite ];

          postInstall = ''
            mkdir -p $out/share/thornhill/docs
            cp docs/rfc-001.md $out/share/thornhill/docs/
          '';

          meta = with pkgs.lib; {
            description = "Ambient cognitive backbone: sensory tap, causal graph, dual-mind reflex engine";
            license = licenses.mit;
            platforms = platforms.linux;
            mainProgram = "thornhilld";
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

            config = lib.mkIf cfg.enable {
              systemd.user.services.thornhilld = {
                description = "Enyxma Thornhill Cognitive Daemon";
                wantedBy = [ "default.target" ];
                serviceConfig = {
                  ExecStart = "${self.packages.${pkgs.system}.default}/bin/thornhilld";
                  Restart = "on-failure";
                  RestartSec = "2s";
                };
              };
            };
          };

        # Evrensel Home Manager Modülü
        homeManagerModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.thornhill;
          in
          {
            options.services.thornhill = {
              enable = lib.mkEnableOption "Enyxma Thornhill Cognitive Daemon";
            };

            config = lib.mkIf cfg.enable {
              systemd.user.services.thornhilld = {
                Unit = {
                  Description = "Enyxma Thornhill Cognitive Daemon";
                  PartOf = [ "default.target" ];
                };
                Service = {
                  ExecStart = "${self.packages.${pkgs.system}.default}/bin/thornhilld";
                  Restart = "on-failure";
                  RestartSec = "2s";
                };
                Install = {
                  WantedBy = [ "default.target" ];
                };
              };
            };
          };
      };
}
