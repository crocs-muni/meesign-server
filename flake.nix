{
  description = "MeeSign Server";

  inputs = {
    nixpkgs.url      = "github:NixOS/nixpkgs/nixos-unstable";
    rust-overlay.url = "github:oxalica/rust-overlay";
    flake-utils.url  = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, rust-overlay, flake-utils, ... }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        overlays = [ (import rust-overlay) ];
        pkgs = import nixpkgs {
          inherit system overlays;
        };
        nativeDependencies = with pkgs; [
            libpq
            pkg-config
            jdk17
            rust-bin.beta.latest.default
            protobuf
          ];
        buildDependencies = with pkgs; [
          openssl
        ];
        meesign-server = pkgs.callPackage ./default.nix { };
      in
      with pkgs;
      {
        devShells.default = mkShell {
          buildInputs = buildDependencies ++ nativeDependencies;
        };

        packages = {
          inherit meesign-server;
          default = meesign-server;
        };
      }
    );
}
