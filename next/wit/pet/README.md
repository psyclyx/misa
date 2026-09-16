# Session pet fixture

This real WIT component keeps `pet.treats` on the session owner. The installed
`pet.feed` command and `pet_feed` model tool reach the same validated handler.
The `pet.feed` action binding supplies one treat; model calls accept an `amount`
between 1 and 10. No client contains pet-specific state or rendering code.

The `plugin.pet.companion` presentation offers a portable status document and a
richer document using the existing semantic meter primitive. The latter requires
`semantic.meter@1`. Both derive from `pet.state`; hiding the presentation removes
its observation without stopping the session or removing the tool.

From `next/` inside the development shell:

```sh
cargo build --manifest-path wit/pet/Cargo.toml --target wasm32-unknown-unknown --release
wasm-tools component new wit/pet/target/wasm32-unknown-unknown/release/policy_pet.wasm -o /tmp/pet.component.wasm
cargo run -p misa-daemon -- --plugin /tmp/pet.component.wasm
```

In the terminal client, `/presentation plugin.pet.companion auto` selects the
best supported variant. `/presentation plugin.pet.companion portable` explicitly
chooses the portable version; `hide` releases the optional observation. `/actions`
opens the actions offered by visible documents, including **Feed**.

The feature-gated plugin integration suite builds this component and checks
command/tool state convergence, capability selection, and local show/hide choices:

```sh
cargo test -p misa-plugin --features guest-fixture --test a_plugin_runs_in_the_loop
```
