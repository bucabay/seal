# Per-item Touch ID on macOS

## Status — October 8, 2026

**Biometric rollout is deferred. Standard OS-keychain authentication remains the
fallback when a biometric-capable build is unavailable.** Secrets remain individual
keychain items; the proposed encrypted-file/master-key design was cancelled.

Completed in the working tree:

- Removed duplicate secret reads from Keymaker's list/health refresh; these use
  item metadata. Reveal reads once, and concurrent reveal/copy clicks share only
  their pending request.
- Existing protected items use native per-item `USER_PRESENCE` authorization.
  Cancellation/denial does not retry through the legacy backend or environment.
- Standard-keychain reads/writes work without biometric provisioning. New
  standard saves use `security -T "" -U` rather than `-A` or delete/add. The empty
  trusted-app list lets macOS request its normal keychain authorization.
- Added native biometric routing, per-item upgrade actions, signing/profile
  validation, and CLI bundle preparation. Physical Touch ID is **not verified**.

The installed PATH binary was `/opt/homebrew/bin/seal` version 0.1.5. It is separate
from these changes. Older items created with `security -A` can still be read
without a prompt because that is their existing OS policy. Selecting the standard
fallback does not itself rewrite every item's ACL; a restrictive save or explicit
upgrade is needed to change an old permissive item.

Signing investigation:

- Two existing Apple Development identities are valid. Reuse is preferred.
- Personal team `YL2QQGZ7PQ` has an expired Developer Program membership. New
  profile creation is blocked; the Offmode team also has a pending agreement.
- Four cached profiles remain unexpired but authorize **iOS**, not native macOS.
  They cannot provision this Tauri/Rust build. No reusable macOS profile was found
  in Xcode's cache, installed owned apps, project folders, or Downloads/Documents/Desktop.
- Signing a raw CLI with restricted keychain entitlements did not produce a
  launchable build. It needs an authorizing profile embedded in an app-like bundle.
- A disposable app-group experiment using the existing identity signed and ran,
  but native protected-item creation returned `-34018` (missing entitlement).
  This route did not establish a working profile-free biometric implementation.

### Deferred follow-up: ship per-item biometric authentication

1. Obtain an existing valid **macOS** profile that authorizes the selected
   certificate, app ID, and shared keychain group; otherwise restore Apple Developer
   access and create one. Do not substitute an iOS profile.
2. Sign/provision CLI, GUI, and broker with the shared group. Preserve the standard
   OS-keychain fallback for users without biometric provisioning.
3. Verify bundled CLI launch and PATH selection before replacing an installation.
4. Test a disposable item interactively: create/upgrade, list/health with no
   prompts, one authorization for reveal/use, cancellation without retry, and
   update/delete. Check both CLI and GUI, including Touch ID/password fallback.
5. Verify cross-binary access and safe legacy-copy cleanup; wire provisioning into
   packaging, Homebrew/release distribution, and installation.

Reference files: `crates/seal-keychain/src/lib.rs`, `src-tauri/src/keychain.rs`,
`keymaker/crates/keymaker-core/src/store.rs`, `scripts/sign-macos.py`, and `install.sh`.

The automated regression checks pass, but they do not establish that the physical
Touch ID dialog works or that a production biometric build has been installed.

Seal and Keymaker keep one keychain item per secret. A protected item uses
Apple's Data Protection Keychain with `USER_PRESENCE` access control and
`WhenUnlockedThisDeviceOnly` accessibility. macOS offers Touch ID when available,
or the device password fallback. The app cannot replace or choose the system
dialog, but its access policy enables that biometric authorization UI.

There is no separate biometric preflight or whole-vault unlock. A read makes
one native password query, which authorizes and retrieves that individual item.
Secret values and authentication contexts are not cached across reads. Concurrent
reveal/copy clicks for the same item share only the in-flight request. A cancelled
read stops immediately; it does not retry through an older item or environment.

## Signed builds

The Data Protection Keychain requires a code-signing identity, an Apple-issued
macOS provisioning profile authorizing the restricted `keychain-access-groups`
entitlement, and an app-like bundle containing that profile. CLI and GUI must use
the same team and shared group. Signing a raw executable with entitlements alone
does **not** work: it has nowhere to embed the authorizing profile, and macOS may
reject it before it runs.

Unsigned/ad-hoc builds use the standard-keychain backend for unprotected items.
Protected-item reads never downgrade after authorization is cancelled or denied.
Explicit biometric upgrades still require provisioning; no secrets are moved if
that preflight fails. Older installed binaries may still use their old prompt-free
policy; changing source does not change those binaries or existing item ACLs.

Build the artifacts normally, then sign them **before** packaging or installing:

```sh
security find-identity -v -p codesigning
/usr/bin/python3 scripts/sign-macos.py \
  --identity "Apple Development: Your Name (...)" \
  --team-id YOURTEAMID \
  --profile /path/to/Seal.provisionprofile \
  src-tauri/target/release/seal \
  src-tauri/target/release/bundle/macos/Seal.app
```

The team ID is the certificate's organizational unit (OU), not necessarily the
parenthesized identifier in its display name. Obtain the matching macOS profile
from Xcode or Apple Developer; it must authorize the certificate, bundle ID, and
`<TEAM>.app.seal.secure.secrets` group. The helper validates all three and expiry
before signing, and embeds the profile in each bundle.

For a CLI input, the helper creates an app-like sibling bundle, for example
`src-tauri/target/release/seal.app`, leaving the original raw executable alone.
Run or symlink **the printed bundled executable path**:

```sh
src-tauri/target/release/seal.app/Contents/MacOS/seal --version
src-tauri/target/release/seal.app/Contents/MacOS/seal protect project/api_key
```

For Keymaker use its matching profile and `--identifier dev.bucabay.keymaker` for
the CLI (`keymaker/target/release/keymaker`) and GUI
(`keymaker/crates/keymaker-gui/target/release/bundle/macos/Keymaker.app`). A profile
covering one bundle ID does not necessarily cover the other. All artifacts share
the group `<TEAM>.app.seal.secure.secrets`. The helper does not enable App Sandbox;
Keymaker's task/broker sandboxing remains separate.

New items saved by a biometric-capable build receive user-presence protection.
Without provisioning, standard keychain saves use normal OS authorization instead.
All processes consuming protected items, including the broker, must run from a
matching provisioned and signed bundle.

## Upgrade an existing item

For biometric source installation, set `SEAL_SIGNING_IDENTITY`, `SEAL_TEAM_ID`,
and `SEAL_PROVISIONING_PROFILE`. The installer provisions the CLI bundle and checks
launch before updating its symlink. Without a profile, it installs the standard
keychain build; an existing identity may optionally sign that binary. Check
`command -v seal` afterwards to detect an older Homebrew binary earlier in PATH.

Click the fingerprint icon beside an item in either GUI. In Seal's CLI:

```sh
seal protect project/api_key
seal protect project/db_url -e production
```

For an inherited item, the GUI upgrades its owning environment. The CLI upgrades
only the explicitly addressed environment. Upgrade reads the old item once,
creates the protected item, then removes the old copy. No plaintext file is
created. That first conversion can require the **old** keychain authorization;
subsequent reads use the biometric-capable native dialog.

Legacy Seal versions stored items with `security -A` (allow any application).
Upgrading removes that legacy item after committing the protected copy. Merely
installing a new binary cannot change an existing item's OS access policy.

If removing the old copy fails, the operation reports failure and the item is
not labelled protected. Retrying finishes cleanup without reading the value
again. Item names, listing, and health checks do not read password data or prompt.

## Verification

Automated tests cover refresh without password reads, a single read for reveal,
and coalescing repeated reveal/copy clicks. Native code compiles against Apple's
Security framework. The physical Touch ID/password dialog needs an interactive
test from a signed build: upgrade a disposable item, refresh without prompts,
reveal once, cancel a subsequent reveal, and update/delete the item. The native
round-trip test is intentionally ignored in unattended test runs.
