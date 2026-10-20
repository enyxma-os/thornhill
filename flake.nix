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
            mkdir -p $out/share/thornhill/docs $out/share/thornhill/shell-hook
            cp docs/rfc-001.md $out/share/thornhill/docs/
            cp shell-hook/thornhill.sh $out/share/thornhill/shell-hook/
          '';

          meta = with pkgs.lib; {
            description = "Ambient cognitive backbone: sensory tap, causal graph, dual-mind reflex engine";
            license = licenses.mit;
            platforms = platforms.linux;
            mainProgram = "thornhilld";
          };
        };
      }) // {
        # Evrensel NixOS Modülü (system düzeyi kullanıcı servisi)
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
                description = "Thornhill paketi (varsayılan: bu flake'in çıktısı)";
              };
            };

            config = lib.mkIf cfg.enable {
              # Kullanıcı başına sistemd servisi: her oturum açan kullanıcı için
              # ayrı thornhilld (RFC-001: kullanıcı başına daemon modeli).
              systemd.user.services.thornhilld = {
                description = "Enyxma Thornhill Cognitive Daemon";
                wantedBy = [ "default.target" ];
                # Aksiyom 2 destek: servisi login hedefine bağla
                partOf = [ "graphical-session.target" ];
                serviceConfig = {
                  ExecStart = "${cfg.package}/bin/thornhilld";
                  Restart = "on-failure";
                  RestartSec = "2s";
                  # Sertleştirme (§8 güvenlik modeliyle uyumlu)
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

        # Evrensel Home Manager Modülü (kullanıcı düzeyi + shell hook entegrasyonu)
        homeManagerModules.default = { config, lib, pkgs, ... }:
          let
            cfg = config.services.thornhill;
          in
          {
            options.services.thornhill = {
              enable = lib.mkEnableOption "Enyxma Thornhill Cognitive Daemon";
              package = lib.mkOption {
                type = lib.types.package;
                default = self.packages.${pkgs.system}.default;
                description = "Thornhill paketi";
              };
              # Shell hook: zsh veya bash rc'ye source satırı ekler (§4.1)
              enableShellHook = lib.mkOption {
                type = lib.types.bool;
                default = false;
                description = ''
                  Kabuk hata kancasını (thornhill.sh) etkileşimli kabuğa bağla.
                  Yalnız exit_code != 0 iken yazar; Thornhill kapalıysa sessizce pas geçer.
                '';
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
              # Shell hook entegrasyonu: HM üzerinden zsh/bash'e source satırı
              (lib.mkIf (cfg.enable && cfg.enableShellHook) {
                programs.zsh.initExtra = lib.mkOrder 1200 ''
                  # Thornhill duyusal kancası (yalnız hata anında yazar)
                  source ${cfg.package}/share/thornhill/shell-hook/thornhill.sh
                '';
                programs.bash.initExtra = lib.mkOrder 1200 ''
                  # Thornhill duyusal kancası (yalnız hata anında yazar)
                  source ${cfg.package}/share/thornhill/shell-hook/thornhill.sh
                '';
              })
            ];
          };
      };
}
