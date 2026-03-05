# Identity API Review

## 1. `identities()` returns raw `Doc` — should return typed data

`session/mod.rs:1227` returns `Doc`, forcing callers to manually deserialize:

```rust
// Current: callers do this ugly dance
let tracked = client_user.identities().await?;
let entry = TrackedIdentity::try_from(tracked.iter().next().unwrap().1).unwrap();
```

**Fix:** Return `Vec<(String, TrackedIdentity)>` (or a `BTreeMap`). The `Doc` is an internal storage detail that leaks through the API.

## 2. `get_identity` is `&self` but silently writes

`session/mod.rs:1120` — This method does two kinds of writes behind a shared reference:

- Legacy backfill (lines 1137-1145)
- Pending → Active transition (lines 1186-1194)

This works because `Database` uses interior mutability, but it's surprising. A `get_*` method that mutates persistent state is a footgun — callers don't expect side effects from a read.

**Fix:** Either make it `&mut self` for honesty, or split into `get_identity(&self)` that only returns Active identities and a separate `check_identity_status(&mut self)` that does the transition.

## 3. Pending path does redundant discovery

`session/mod.rs:1167-1182` — When `tracked.key_id` is `Some`, the code resolves `key_id` from it (line 1170), then *still* calls `identity_key_discover` (line 1178) just to check whether the key is visible in auth settings. This opens and scans the identity DB a second time for a check that `open_database_with_key` on line 1183 would catch anyway (it fails if the key isn't there).

**Fix:** Drop the redundant `identity_key_discover` guard. Let `open_database_with_key` be the single check. If it fails, return `None` — which is already what happens on line 1212.

## 4. `identity_key_discover` passes empty name in error

`session/mod.rs:1331-1332`:

```rust
Err(UserError::NoKeyInIdentity {
    name: String::new(),  // <-- lost context
    identity_id: identity_root_id.clone(),
})
```

The caller (`identity_key` at line 1306) catches this and re-wraps with the correct name, but `get_identity`'s Pending path (line 1171) doesn't — if discovery fails there, the error gets swallowed into `None`. The empty name is still a latent bug for any future caller.

**Fix:** Either take `name: &str` as a parameter to `identity_key_discover`, or have it return a distinct error type that doesn't need a name.

## 5. Inconsistent ownership across the API

| Method | Parameter | Takes |
|---|---|---|
| `create_identity` | `key_id` | `&PublicKey` |
| `Identity::set_key` | `key_id` | `PublicKey` (owned) |
| `Identity::add_key` | `pubkey` | `&PublicKey` |
| `Identity::add_key` | `auth_key` | `AuthKey` (owned) |
| `register_identity` | `auth_key` | `&AuthKey` |

**Fix:** Pick a convention. Since `PublicKey` and `AuthKey` are both small/`Clone`, taking `&` everywhere is natural. `set_key` taking owned makes sense since it stores them, but then `register_identity` should also take owned `AuthKey` since it passes it through.

## 6. `Identity::open_database` silently picks first SigKey

`identity.rs:126-133` — Takes the first result from `find_sigkeys` with no indication to the caller what permission level was resolved.

**Fix:** Either return `(Database, Permission)` so callers know what they got, or at minimum document that it picks the highest-permission path. A more robust version would let callers specify a minimum required permission.
