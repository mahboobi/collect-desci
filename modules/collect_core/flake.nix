{
  description = "Collect Core — documents, signing, validation, media and indexing";

  inputs = {
    # Pinned to 0.3.0: basecamp 0.3.0 speaks logos-protocol 0.9, and a module
    # built against 0.2.x may install cleanly but have its calls refused once
    # loaded. 0.3.1 is already ahead of what basecamp 0.3.0 links.
    logos-module-builder.url = "github:logos-co/logos-module-builder/0.3.0";

    # Declared so the host AUTO-LOADS them. Calling a module by name is not
    # enough: with no declaration nothing loads storage_module or
    # delivery_module at all, and every call came back
    # `object_unavailable` — "module not loaded, not published, or transport
    # failure" — with the event subscriptions sitting DEFERRED. The
    # declaration is what makes them appear.
    #
    # Each also publishes `packages.<system>.lidl`, which is what the builder
    # demands of a declared dependency: naming one that does not fails the
    # build outright (docs/api-verification.md round 4).
    storage_module.url = "github:logos-co/logos-storage-module";
    delivery_module.url = "github:logos-co/logos-delivery-module";

    # Declared for its side effect: making the host load it.
    #
    # Collect never calls this module. `delivery_module` does — it declares
    # `optional_depends [libp2p_module, liblogos_rln_module]` and bridges RLN
    # proof generation to it. But `optional_depends` does **not** cause
    # loading, and a declaration here does, so this is the lever available.
    # Without it delivery reports
    #
    #   RLN backend did not answer the startup membership check; sends and
    #   proof validation fail until it does
    #
    # and every announcement on logos.test is refused. The dependency is real
    # even though the call path is not: Collect's announcements need RLN.
    # Pinned to the revision logos-delivery-module itself pins, so the bridge
    # and the backend cannot disagree.
    liblogos_rln_module.url =
      "git+https://github.com/logos-co/logos-rln-modules?dir=logos-rln-module&rev=a814e7feda974a16eaa5dd50667b8fd843177f0e";

    # Unify their transitive builder onto our pin, as collect_ui does for
    # collect_core. Without this the lock carries two and the stale one wins
    # under scaffold's --override-input.
    storage_module.inputs.logos-module-builder.follows = "logos-module-builder";
    delivery_module.inputs.logos-module-builder.follows = "logos-module-builder";
    liblogos_rln_module.inputs.logos-module-builder.follows = "logos-module-builder";
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
