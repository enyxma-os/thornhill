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
            echo "Anayasa: docs/rfc-001.md | Doğrulama: just verify | Web: http://127.0.0.1:9099"
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

          # NOT (Faz 6): derleme "build.rs - dashboard" işlemi yapmaz; statik dosyalar
          # include_bytes! ile binary'e gömülüdür (dashboard/ kaynak ağacında durur).
          # Yine de XDG override dizinine kopyalanabilir kaynak olarak paketlenir:
          postInstall = ''
            mkdir -p $out/share/thornhill/docs $out/share/thornhill/shell-hook $out/share/thornhill/shell $out/share/thornhill/dashboard
            cp docs/rfc-001.md docs/DEPLOY.md $out/share/thornhill/docs/
            cp shell-hook/thornhill.sh shell-hook/thornhill.fish $out/share/thornhill/shell-hook/
            cp -r shell/* $out/share/thornhill/shell/
            cp -r dashboard/* $out/share/thornhill/dashboard/
          '';

          meta = with pkgs.lib; {
            description = "Ambient cognitive backbone: sensory tap, causal graph, dual-mind reflex engine, ambient web console";
            license = licenses.mit;
            platforms = platforms.linux;
            mainProgram = "thornhilld";
          };
        };
      }) // {
        nixosModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.thornhill;
          in
          {
            options.services.thornhill = {
              enable = lib.mkEnableOption "Enyxma Thornhill Cognitive Daemon";
              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
              };
            };

            config = lib.mkIf cfg.enable {
              systemd.user.services.thornhilld = {
                description = "Enyxma Thornhill Cognitive Daemon";
                wantedBy = [ "default.target" ];
                partOf = [ "graphical-session.target" ];
                serviceConfig = {
                  ExecStart = "${cfg.package}/bin/thornhilld";
                  Restart = "on-failure";
                  RestartSec = "2s";
                  PrivateTmp = true;
                  NoNewPrivileges = true;
                  ProtectSystem = "strict";
                  ProtectHome = "read-only";
                  ReadWritePaths = [ "%t/thornhill" "%h/.local/state/thornhill" ];
                };
                environment = {
                  XDG_RUNTIME_DIR = "%t";
                  XDG_STATE_HOME = "%h/.local/state";
                };
              };
            };
          };

        homeManagerModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.thornhill;
            shellHookSh = ''
              source ${cfg.package}/share/thornhill/shell-hook/thornhill.sh
            '';
            shellHookFish = ''
              source ${cfg.package}/share/thornhill/shell-hook/thornhill.fish
            '';
          in
          {
            options.services.thornhill = {
              enable = lib.mkEnableOption "Enyxma Thornhill Cognitive Daemon";
              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
              };
              enableShellHook = lib.mkOption {
                type = lib.types.bool;
                default = false;
              };
            };

            config = lib.mkMerge [
              (lib.mkIf cfg.enable {
                systemd.user.services.thornhilld = {
                  Unit = {
                    Description = "Enyxma Thornhill Cognitive Daemon";
                    PartOf = [ "graphical-session.target" ];
                  };
                  Service = {
                    ExecStart = "${cfg.package}/bin/thornhilld";
                    Restart = "on-failure";
                    RestartSec = "2s";
                    NoNewPrivileges = true;
                    PrivateTmp = true;
                  };
                  Install = {
                    WantedBy = [ "default.target" ];
                  };
                };
              })
              (lib.mkIf (cfg.enable && cfg.enableShellHook) {
                programs.fish.interactiveShellInit = lib.mkOrder 1200 shellHookFish;
                programs.zsh.initExtra = lib.mkOrder 1200 shellHookSh;
                programs.bash.initExtra = lib.mkOrder 1200 shellHookSh;
              })
            ];
          };
      };
}
