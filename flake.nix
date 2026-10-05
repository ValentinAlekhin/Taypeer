{
  description = "Taypeer desktop for Linux";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/a7868a727837f3c09cee2ce0ca671c76b1589fed";
  outputs = { self, nixpkgs }:
    let
      system = "x86_64-linux";
      pkgs = import nixpkgs { inherit system; };
      libraries = with pkgs; [ wayland libxkbcommon libxcb fontconfig freetype vulkan-loader libGL glib openssl ];
      fontsConf = pkgs.makeFontsConf {
        fontDirectories = [ (pkgs.ibm-plex.override { families = [ "sans" "mono" ]; }) pkgs.noto-fonts pkgs.noto-fonts-color-emoji ];
      };
      package = pkgs.rustPlatform.buildRustPackage {
        pname = "taypeer";
        version = "0.1.0";
        src = pkgs.lib.cleanSourceWith {
          src = ./.;
          filter = path: type:
            let
              name = builtins.baseNameOf path;
              relative = pkgs.lib.removePrefix (toString ./. + "/") path;
              top = builtins.head (pkgs.lib.splitString "/" relative);
              buildSource = builtins.elem top [ "Cargo.toml" "Cargo.lock" "rust-toolchain.toml" ".cargo" "apps" "crates" "resources" "wireframes" ];
            in (path == toString ./. || buildSource)
              && !(builtins.elem name [ ".git" ".cache" ".agents" ".codex" "target" "ref" "node_modules" "artifacts" "result" ]);
        };
        cargoLock = { lockFile = ./Cargo.lock; allowBuiltinFetchGit = true; };
        nativeBuildInputs = with pkgs; [ pkg-config makeWrapper imagemagick ];
        buildInputs = libraries;
        cargoBuildFlags = [ "-p" "taypeer" "-p" "taypeer-cli" "--bin" "taypeer" "--bin" "taypeer-cli" ];
        # UI fixture workers and public demo credentials are gated by ui-test-support.
        doCheck = false;
        postInstall = ''
          install -Dm644 ${./nix/io.taypeer.Taypeer.desktop} $out/share/applications/io.taypeer.Taypeer.desktop
          mkdir -p $out/share/icons/hicolor/256x256/apps
          magick ${./wireframes/assets/taypeer-app-icon-v1.png} -resize 256x256 $out/share/icons/hicolor/256x256/apps/io.taypeer.Taypeer.png
        '';
        postFixup = ''
          wrapProgram $out/bin/taypeer \
            --suffix LD_LIBRARY_PATH : ${pkgs.lib.makeLibraryPath libraries} \
            --set FONTCONFIG_FILE ${fontsConf}
        '';
        meta = { description = "Taypeer password database manager"; mainProgram = "taypeer"; platforms = [ system ]; };
      };
    in {
      packages.${system} = { default = package; taypeer = package; };
      apps.${system}.default = { type = "app"; program = "${package}/bin/taypeer"; };
      devShells.${system}.default = pkgs.mkShell {
        nativeBuildInputs = with pkgs; [ cargo rustc clippy rustfmt pkg-config pnpm ];
        buildInputs = libraries;
        LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath libraries;
        FONTCONFIG_FILE = fontsConf;
      };
    };
}
