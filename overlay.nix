final: prev: {
  topclaw-web = final.callPackage ./web/package.nix { };

  topclaw = final.callPackage ./package.nix {
    rustToolchain = final.fenix.stable.withComponents [
      "cargo"
      "clippy"
      "rust-src"
      "rustc"
      "rustfmt"
    ];
  };
}
