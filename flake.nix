{
  description = "Nix flake for dbtui — a terminal-based database client";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-26.05";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs =
    {
      self,
      nixpkgs,
      flake-utils,
    }:
    flake-utils.lib.eachDefaultSystem (
      system:
      let
        pkgs = import nixpkgs { inherit system; };

        buildInputs = with pkgs; [
          # TLS / OpenSSL
          openssl
          # Oracle driver (ODPI-C)
          libaio
          # Clipboard support (X11 / Wayland)
          libX11
          libxcb
          wayland
          libxkbcommon
        ];

        # Bind the package once so `packages` and `apps` can share it.
        dbtui = pkgs.rustPlatform.buildRustPackage {
          pname = "dbtui";
          version = "0.4.0";
          src = ./.;

          cargoLock.lockFile = ./Cargo.lock;

          nativeBuildInputs = [ pkgs.pkg-config ];
          inherit buildInputs;

          meta = with pkgs.lib; {
            description = "Terminal database client with Vim-style navigation";
            homepage = "https://github.com/Di3go0-0/dbtui";
            license = licenses.mit;
            maintainers = [
              "sergioia-dev"
              "Di3go0-0"
            ];
          };
        };
      in
      {
        # Development shell — `nix develop`, then `cargo run`
        devShells.default = pkgs.mkShell {
          name = "dbtui-dev";
          buildInputs =
            buildInputs
            ++ (with pkgs; [
              cargo
              rustc
              rust-analyzer
              clippy
              pkg-config
              wl-clipboard
              xclip
              xsel
            ]);
        };

        # `nix build` → result/bin/dbtui, built from the local checkout
        packages.default = dbtui;

        # `nix run` → build + run dbtui
        apps.default = {
          type = "app";
          program = "${dbtui}/bin/dbtui";
        };
      }
    );
}
