{
  description = "A flake to run the scenario URL generator";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  };

  outputs = { self, nixpkgs }:
    let
      supportedSystems = [ "x86_64-linux" "aarch64-linux" "x86_64-darwin" "aarch64-darwin" ];
      forAllSystems = nixpkgs.lib.genAttrs supportedSystems;
      pkgsFor = system: nixpkgs.legacyPackages.${system};
    in
    {
      devShells = forAllSystems (system: {
        default = (pkgsFor system).mkShell {
          buildInputs = [
            ((pkgsFor system).python3.withPackages (ps: [ ps.pyyaml ps.requests ]))
          ];
        };
      });

      apps = forAllSystems (system: {
        default = {
          type = "app";
          program = let
            pkgs = pkgsFor system;
            pythonEnv = pkgs.python3.withPackages (ps: [ ps.pyyaml ps.requests ]);
          in "${pkgs.writeShellScriptBin "generate-urls" ''
            exec ${pythonEnv}/bin/python3 ${self}/generate_urls.py
          ''}/bin/generate-urls";
        };
      });
    };
}
