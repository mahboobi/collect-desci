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


    # Unify their transitive builder onto our pin, as collect_ui does for
    # collect_core. Without this the lock carries two and the stale one wins
    # under scaffold's --override-input.
    storage_module.inputs.logos-module-builder.follows = "logos-module-builder";
    delivery_module.inputs.logos-module-builder.follows = "logos-module-builder";
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
