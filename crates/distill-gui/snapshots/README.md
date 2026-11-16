# GUI snapshots

One pair per scene in `src/fixture.rs` (`SCENES`), all synthetic data:

- `<scene>.txt`: the element tree (keys, roles and the text shown). `cargo test -p distill-gui`
  renders every scene headless and fails when one differs. After an intended change, accept the
  new screens with `DISTILL_UPDATE_SNAPSHOTS=1 cargo test -p distill-gui`.
- `<scene>.png`: the same scene as a picture, for people. Not checked by tests. Regenerate after
  a visual change (a headless run reads no home and starts no worker, but point both at a
  temporary folder anyway):

```sh
cargo build -p distill-gui
t=$(mktemp -d)
for s in $(HOME=$t DISTILL_HOME=$t target/debug/distill-gui --list-scenes); do
  HOME=$t DISTILL_HOME=$t target/debug/distill-gui --snap crates/distill-gui/snapshots/$s.png --scene $s --scale 1
done
```
