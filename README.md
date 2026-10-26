# Mugonat Portal

Designed for 64-bit Linux desktops from Ubuntu 20.04 and Fedora 32 onward. Requires NetworkManager and a desktop keyring. Releases include an AppImage, Debian package, and RPM.

Supports `Mugonat Systems 5.0` and `Mugonat Systems 2.4`. Sign-in details are stored in your desktop keyring. Forms the app cannot handle open in your browser for manual sign-in.

## Install

GitHub Releases provide an AppImage, Debian package, and RPM. In Settings, turn on **Start when you sign in** to keep the app running in the background.

## Versioning and builds

The version in `Cargo.toml` is the app version shown in Settings and used by the packages. Bump it when you choose to publish a new app version; regular fixes and builds do not need a version bump.

Every push to `main` builds the AppImage, Debian package, and RPM. After a successful build, the workflow publishes a GitHub release and creates a tag in this format:

```text
v<app-version>-build<run-number>.<attempt>
```

For example, `v0.1.2-build9.1` is app version `0.1.2`, workflow run `9`, first attempt. A rerun of the same workflow increments the attempt number. Each release tag points to the commit that was built. The app shows both its version and build number in Settings.
