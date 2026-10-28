{
  description = "Collect UI — QML view over collect_core";

  inputs = {
    # Pinned to 0.3.0: basecamp 0.3.0 speaks logos-protocol 0.9, and a module
    # built against 0.2.x may install but have its calls refused. 0.3.1 is
    # already ahead of what basecamp 0.3.0 links.
    logos-module-builder.url = "github:logos-co/logos-module-builder/0.3.0";

    # Declared as an input so scaffold reads an authoritative pin from this
    # flake's lock rather than falling back to its own default.
    collect_core.url = "path:../collect_core";

    # Unify the transitive logos-module-builder onto our pin. Without this the
    # lock carries two, and the stale one wins under scaffold's
    # --override-input: `nix build .#lgx` works here but fails through lgs.
    collect_core.inputs.logos-module-builder.follows = "logos-module-builder";
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosQmlModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
