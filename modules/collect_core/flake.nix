{
  description = "Collect Core — documents, signing, validation, media and indexing";

  inputs = {
    # Pinned to 0.3.0: basecamp 0.3.0 speaks logos-protocol 0.9, and a module
    # built against 0.2.x may install cleanly but have its calls refused once
    # loaded. 0.3.1 is already ahead of what basecamp 0.3.0 links.
    logos-module-builder.url = "github:logos-co/logos-module-builder/0.3.0";
  };

  outputs = inputs@{ logos-module-builder, ... }:
    logos-module-builder.lib.mkLogosModule {
      src = ./.;
      configFile = ./metadata.json;
      flakeInputs = inputs;
    };
}
