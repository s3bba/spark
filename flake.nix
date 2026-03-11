{
  inputs = {
    nixpkgs.url = "github:nixos/nixpkgs?ref=nixos-unstable";
    rust = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
  };

  outputs =
    {
      self,
      nixpkgs,
      rust,
    }:
    let
      lib = nixpkgs.lib;
      supportedSystems = [
        "x86_64-linux"
        "aarch64-linux"
      ];
      forAllSystems = lib.genAttrs supportedSystems;

      mkPkgs =
        system:
        import nixpkgs {
          inherit system;
          overlays = [
            rust.overlays.default
          ];
        };

      runtimeLibs = pkgs: [
        pkgs.freetype
        pkgs.wayland
        pkgs.libxkbcommon
      ];

      mkRustToolchain = pkgs: pkgs.rust-bin.fromRustupToolchainFile ./rust-toolchain.toml;

      mkSpark =
        pkgs:
        let
          rustToolchain = mkRustToolchain pkgs;
          rustPlatform = pkgs.makeRustPlatform {
            cargo = rustToolchain;
            rustc = rustToolchain;
          };
        in
        rustPlatform.buildRustPackage {
          pname = "spark";
          version = "0.0.0";
          src = lib.cleanSource ./.;
          cargoLock.lockFile = ./Cargo.lock;
          SPARK_EMBEDDED_FONT_FILE =
            "${pkgs.ibm-plex}/share/fonts/opentype/IBMPlexMono-Regular.otf";

          nativeBuildInputs = [
            pkgs.pkg-config
          ];

          buildInputs = runtimeLibs pkgs;

          meta = {
            description = "Wayland launcher";
            mainProgram = "spark";
            platforms = supportedSystems;
          };
        };
    in
    {
      packages = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          spark = mkSpark pkgs;
        in
        {
          default = spark;
          spark = spark;
        }
      );

      apps = forAllSystems (
        system:
        let
          defaultApp = {
            type = "app";
            program = "${self.packages.${system}.default}/bin/spark";
          };
        in
        {
          default = defaultApp;
          spark = defaultApp;
        }
      );

      overlays.default = lib.composeExtensions rust.overlays.default (final: prev: {
        spark = mkSpark final;
      });

      devShells = forAllSystems (
        system:
        let
          pkgs = mkPkgs system;
          rustToolchain = mkRustToolchain pkgs;
        in
        {
          default = pkgs.mkShell {
            buildInputs = [
              rustToolchain
              pkgs.cargo-edit
              pkgs.wild
              pkgs.clang
              pkgs.pkg-config
              pkgs.ibm-plex
            ] ++ runtimeLibs pkgs;
            env.LD_LIBRARY_PATH = pkgs.lib.makeLibraryPath (runtimeLibs pkgs);
            env.RUST_SRC_PATH = "${rustToolchain}/lib/rustlib/src/rust/library";
            env.SPARK_EMBEDDED_FONT_FILE =
              "${pkgs.ibm-plex}/share/fonts/opentype/IBMPlexMono-Regular.otf";
            env.SPARK_FONT_FILE = "${pkgs.ibm-plex}/share/fonts/opentype/IBMPlexMono-Regular.otf";
          };
        }
      );
    };
}
