# Releasing

Push a version tag and CI does the rest:

```sh
git tag v0.2.0 && git push origin v0.2.0
```

Every platform is built, signed where the secrets below exist, and put in a **draft** release
with a `SHA256SUMS` file. Check the draft, then publish it. People can check a download with
`sha256sum -c SHA256SUMS --ignore-missing` (`shasum -a 256 -c` on macOS).

Without the secrets, builds still come out: Windows unsigned (SmartScreen warns) and macOS
ad-hoc signed (Gatekeeper asks the first time it opens).

## Secrets

Add these under the repository's **Settings → Secrets and variables → Actions**. Signing turns
itself on once the first secret of each pair exists. Pull requests from forks never get them.

### Windows: a code-signing certificate

| Secret | What |
| --- | --- |
| `WINDOWS_CERTIFICATE` | The certificate and key as a `.pfx`, base64 encoded: `base64 -w0 panna.pfx` |
| `WINDOWS_CERTIFICATE_PASSWORD` | The `.pfx` file's password |

Any code-signing certificate from a public authority works. Since 2023 these come on a
hardware token or a cloud signing service rather than as a file; for those, swap the
signing step for the provider's own (for example Azure Trusted Signing's GitHub action).

### macOS: a Developer ID and notarisation

Needs an Apple Developer Program membership.

| Secret | What |
| --- | --- |
| `MACOS_CERTIFICATE` | A **Developer ID Application** certificate and key exported from Keychain Access as `.p12`, base64 encoded: `base64 -i panna.p12` |
| `MACOS_CERTIFICATE_PASSWORD` | The `.p12` file's password |
| `APPLE_ID` | The Apple ID email of the developer account |
| `APPLE_TEAM_ID` | The 10-character team ID (Membership details on developer.apple.com) |
| `APPLE_APP_PASSWORD` | An app-specific password for that Apple ID, made at appleid.apple.com |

The app is signed with the hardened runtime, sent to Apple's notary service, and the ticket is
stapled to it, so it opens offline without a warning.

### Linux

The tarball and Flatpak aren't signed; the checksums cover them. Flathub signs what it builds
itself.
